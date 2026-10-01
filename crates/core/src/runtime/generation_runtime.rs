use std::{
    path::{Path, PathBuf},
    process::Stdio,
};

use parking_lot::Mutex;
use thiserror::Error;
use tokio::process::{Child, Command};

use crate::errors::{CodedError, ErrorCode};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ModelArtifact {
    pub filename: &'static str,
    pub repository: &'static str,
    pub revision: &'static str,
    pub remote_path: &'static str,
    pub size_bytes: u64,
    pub sha256: &'static str,
}

/// The single generation profile the app runs: Qwen-Image-2.1, served by the
/// native `stable-diffusion.cpp` `sd-server`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GenerationProfile {
    pub id: &'static str,
    /// The directory under `model_dir` the artifacts are provisioned into.
    pub model_root: &'static str,
    pub diffusion: ModelArtifact,
    pub text_encoder: ModelArtifact,
    pub vae: ModelArtifact,
}

const QWEN_DIFFUSION: ModelArtifact = ModelArtifact {
    filename: "qwen-image-2.1-UC-int8_convrot.safetensors",
    repository: "Comfy-Org/Qwen-Image-2.1",
    revision: "main",
    remote_path: "split_files/diffusion_models/qwen-image-2.1-UC-int8_convrot.safetensors",
    size_bytes: 7_256_796_840,
    sha256: "5bc5a6c007eff1e0d4004344a24c4af966b9d2c83d142e613b0d456ccceb19ae",
};
const QWEN_TEXT_ENCODER: ModelArtifact = ModelArtifact {
    filename: "qwen3vl_8b_int8_convrot.safetensors",
    repository: "Comfy-Org/Qwen-Image-2.1",
    revision: "main",
    remote_path: "text_encoders/qwen3vl_8b_int8_convrot.safetensors",
    size_bytes: 9_350_798_360,
    sha256: "8bfd0f6e12abf2d2d697ecc888e5e90b0d6741d6708f05799f53afa560452e8f",
};
const QWEN_VAE: ModelArtifact = ModelArtifact {
    filename: "qwen_image_2.1_vae_bf16.safetensors",
    repository: "Comfy-Org/Qwen-Image-2.1",
    revision: "main",
    remote_path: "vae/qwen_image_2.1_vae_bf16.safetensors",
    size_bytes: 675_509_688,
    sha256: "bb21f7473051e1ac368515dd3f2e15cd44d7a11748ee8823e1ddca3e4876b7c9",
};

pub const fn generation_profile() -> GenerationProfile {
    GenerationProfile {
        id: "qwen-image-2.1",
        model_root: "qwen-image-2.1",
        diffusion: QWEN_DIFFUSION,
        text_encoder: QWEN_TEXT_ENCODER,
        vae: QWEN_VAE,
    }
}

pub struct GenerationRuntime {
    executable: PathBuf,
    model_directory: PathBuf,
    lora_directory: PathBuf,
    child: Mutex<Option<Child>>,
}

#[derive(Debug, Error)]
pub enum GenerationRuntimeError {
    #[error("native generation runtime is already running")]
    AlreadyRunning,
    #[error("sd-server executable not found at {0}")]
    MissingExecutable(PathBuf),
    #[error("model artifact not found at {0}")]
    MissingArtifact(PathBuf),
    #[error("failed to create LoRA directory {path}: {source}")]
    CreateLoraDirectory {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to start native generation runtime: {0}")]
    Start(#[source] std::io::Error),
    #[error("failed while stopping native generation runtime: {0}")]
    Stop(#[source] std::io::Error),
}

impl CodedError for GenerationRuntimeError {
    fn code(&self) -> ErrorCode {
        match self {
            Self::AlreadyRunning => ErrorCode::RuntimeGenerationAlreadyRunning,
            Self::MissingExecutable(_) => ErrorCode::RuntimeGenerationMissingExecutable,
            Self::MissingArtifact(_) => ErrorCode::RuntimeGenerationMissingArtifact,
            Self::CreateLoraDirectory { .. } => ErrorCode::RuntimeGenerationCreateLoraDirectory,
            Self::Start(_) => ErrorCode::RuntimeGenerationStart,
            Self::Stop(_) => ErrorCode::RuntimeGenerationStop,
        }
    }
}

impl GenerationRuntime {
    pub fn new(
        executable: impl Into<PathBuf>,
        model_directory: impl Into<PathBuf>,
        lora_directory: impl Into<PathBuf>,
    ) -> Self {
        Self {
            executable: executable.into(),
            model_directory: model_directory.into(),
            lora_directory: lora_directory.into(),
            child: Mutex::new(None),
        }
    }

    /// Starts the resident server holding the Qwen-Image-2.1 profile.
    pub fn start(&self, host: &str, port: u16) -> Result<(), GenerationRuntimeError> {
        let mut child_slot = self.child.lock();
        if child_slot.is_some() {
            return Err(GenerationRuntimeError::AlreadyRunning);
        }
        if !self.executable.is_file() {
            return Err(GenerationRuntimeError::MissingExecutable(
                self.executable.clone(),
            ));
        }
        let profile = generation_profile();
        let model_root = self.model_directory.join(profile.model_root);
        for artifact in [profile.diffusion, profile.text_encoder, profile.vae] {
            let path = model_root.join(artifact.filename);
            if !path.is_file() {
                return Err(GenerationRuntimeError::MissingArtifact(path));
            }
        }
        std::fs::create_dir_all(&self.lora_directory).map_err(|source| {
            GenerationRuntimeError::CreateLoraDirectory {
                path: self.lora_directory.clone(),
                source,
            }
        })?;
        // sd.cpp registers only the top-level files of its lora directory;
        // link subdirectory adapters to the top level before the server's
        // registration scan runs at startup.
        let linked = crate::registry::prepare_lora_links(&self.lora_directory);
        if linked > 0 {
            tracing::info!(count = linked, dir = %self.lora_directory.display(), "linked subdirectory loras for the native runtime");
        }

        let child = self
            .command(profile, &model_root, host, port)
            .spawn()
            .map_err(GenerationRuntimeError::Start)?;
        tracing::info!(
            pid = child.id(),
            profile = profile.id,
            "started resident native generation runtime"
        );
        *child_slot = Some(child);
        Ok(())
    }

    /// Starts the server unless an instance this runtime owns is alive.
    pub async fn ensure_running(
        &self,
        host: &str,
        port: u16,
    ) -> Result<(), GenerationRuntimeError> {
        if self.is_running() {
            return Ok(());
        }
        self.start(host, port)
    }

    pub async fn stop(&self) -> Result<(), GenerationRuntimeError> {
        let child = self.child.lock().take();
        if let Some(mut child) = child {
            child.start_kill().map_err(GenerationRuntimeError::Stop)?;
            child.wait().await.map_err(GenerationRuntimeError::Stop)?;
            tracing::info!("stopped native generation runtime");
        }
        Ok(())
    }

    pub fn is_running(&self) -> bool {
        let mut slot = self.child.lock();
        let Some(child) = slot.as_mut() else {
            return false;
        };
        match child.try_wait() {
            Ok(None) => true,
            Ok(Some(status)) => {
                tracing::warn!(?status, "native generation runtime exited");
                slot.take();
                false
            }
            Err(error) => {
                tracing::warn!(%error, "could not inspect native generation runtime");
                false
            }
        }
    }

    pub fn executable(&self) -> &Path {
        &self.executable
    }

    /// Whether every artifact the profile needs exists under the model directory.
    pub fn has_profile_artifacts(&self) -> bool {
        let profile = generation_profile();
        let model_root = self.model_directory.join(profile.model_root);
        [profile.diffusion, profile.text_encoder, profile.vae]
            .iter()
            .all(|artifact| model_root.join(artifact.filename).is_file())
    }

    fn command(
        &self,
        profile: GenerationProfile,
        model_root: &Path,
        host: &str,
        port: u16,
    ) -> Command {
        let mut command = Command::new(&self.executable);
        command
            .arg("--diffusion-model")
            .arg(model_root.join(profile.diffusion.filename))
            .arg("--llm")
            .arg(model_root.join(profile.text_encoder.filename))
            .arg("--vae")
            .arg(model_root.join(profile.vae.filename))
            // An explicit quantized prefix cache keeps the working set small;
            // `auto` would pick f32 (4 GiB per condition) with flash attention.
            .arg("--model-args")
            .arg("qwen_image_2_1_prefix_cache_type=q8_0")
            .arg("--diffusion-fa")
            .arg("--lora-model-dir")
            .arg(&self.lora_directory)
            .arg("--listen-ip")
            .arg(host)
            .arg("--listen-port")
            .arg(port.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        command
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{generation_profile, GenerationRuntime};

    #[test]
    fn profile_provisions_the_qwen_artifacts() {
        let profile = generation_profile();
        assert_eq!(profile.id, "qwen-image-2.1");
        assert_eq!(
            profile.diffusion.filename,
            "qwen-image-2.1-UC-int8_convrot.safetensors"
        );
        assert_eq!(profile.text_encoder.size_bytes, 9_350_798_360);
        assert_eq!(profile.vae.size_bytes, 675_509_688);
    }

    #[test]
    fn profile_artifacts_are_detected_on_disk() {
        let directory =
            std::env::temp_dir().join(format!("svs-artifacts-test-{}", uuid::Uuid::new_v4()));
        let runtime = GenerationRuntime::new("/dev/null/sd-server", &directory, &directory);

        assert!(!runtime.has_profile_artifacts());

        let profile = generation_profile();
        let model_root = directory.join(profile.model_root);
        fs::create_dir_all(&model_root).expect("model root should be created");
        for artifact in [profile.diffusion, profile.text_encoder, profile.vae] {
            fs::write(model_root.join(artifact.filename), b"x")
                .expect("artifact should be written");
        }

        assert!(runtime.has_profile_artifacts());
        let _ = fs::remove_dir_all(&directory);
    }
}
