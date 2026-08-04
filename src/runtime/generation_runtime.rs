use std::{
    path::{Path, PathBuf},
    process::Stdio,
};

use parking_lot::Mutex;
use thiserror::Error;
use tokio::process::{Child, Command};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KreaQuantization {
    Q2,
    Q4,
}

impl KreaQuantization {
    pub const fn profile_id(self) -> &'static str {
        match self {
            Self::Q2 => "krea-2-turbo-q2",
            Self::Q4 => "krea-2-turbo-q4",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ModelArtifact {
    pub filename: &'static str,
    pub repository: &'static str,
    pub revision: &'static str,
    pub remote_path: &'static str,
    pub size_bytes: u64,
    pub sha256: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KreaProfile {
    pub id: &'static str,
    pub quantization: &'static str,
    pub diffusion: ModelArtifact,
    pub text_encoder: ModelArtifact,
    pub vae: ModelArtifact,
}

const QWEN3_VL_Q4: ModelArtifact = ModelArtifact {
    filename: "Qwen3VL-4B-Instruct-Q4_K_M.gguf",
    repository: "Qwen/Qwen3-VL-4B-Instruct-GGUF",
    revision: "1cd86afb9a95c410a6038ab3b40d8b578c892266",
    remote_path: "Qwen3VL-4B-Instruct-Q4_K_M.gguf",
    size_bytes: 2_497_281_664,
    sha256: "66358cb18bb6b3b1b6675aa412c7a88ef01d228f481184d13668e5201c730a0a",
};
const KREA_VAE: ModelArtifact = ModelArtifact {
    filename: "wan_2.1_vae.safetensors",
    repository: "Comfy-Org/Wan_2.1_ComfyUI_repackaged",
    revision: "06e001fc51048fb03433a6fb25334de7836704a5",
    remote_path: "split_files/vae/wan_2.1_vae.safetensors",
    size_bytes: 253_815_318,
    sha256: "2fc39d31359a4b0a64f55876d8ff7fa8d780956ae2cb13463b0223e15148976b",
};

pub const fn krea_profile(quantization: KreaQuantization) -> KreaProfile {
    let (id, name, size, sha256) = match quantization {
        KreaQuantization::Q2 => (
            "krea-2-turbo-q2",
            "krea2_turbo-q2_k.gguf",
            4_212_730_912,
            "eb9f3ad08e552dc9244a1c18dc2def02fbaaca77c7fab457de50ba47720694a6",
        ),
        KreaQuantization::Q4 => (
            "krea-2-turbo-q4",
            "krea2_turbo-iq4_xs.gguf",
            6_816_424_992,
            "56e1bfb0318693e4d0882e48c72286b7ad98f72dc9c9e5c46a5164c6cca7c77d",
        ),
    };
    KreaProfile {
        id,
        quantization: match quantization {
            KreaQuantization::Q2 => "Q2_K",
            KreaQuantization::Q4 => "IQ4_XS",
        },
        diffusion: ModelArtifact {
            filename: name,
            repository: "gguf-org/krea-2-gguf",
            revision: "7813603b1acf32759db87950268afb7e61b362b1",
            remote_path: name,
            size_bytes: size,
            sha256,
        },
        text_encoder: QWEN3_VL_Q4,
        vae: KREA_VAE,
    }
}

pub struct GenerationRuntime {
    executable: PathBuf,
    model_directory: PathBuf,
    lora_directory: PathBuf,
    child: Mutex<Option<Child>>,
    active_profile: Mutex<Option<KreaQuantization>>,
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
            active_profile: Mutex::new(None),
        }
    }

    pub fn start(
        &self,
        quantization: KreaQuantization,
        host: &str,
        port: u16,
    ) -> Result<(), GenerationRuntimeError> {
        let mut child_slot = self.child.lock();
        if child_slot.is_some() {
            return Err(GenerationRuntimeError::AlreadyRunning);
        }
        if !self.executable.is_file() {
            return Err(GenerationRuntimeError::MissingExecutable(
                self.executable.clone(),
            ));
        }
        let profile = krea_profile(quantization);
        let model_root = self.model_directory.join("krea-2");
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
        *self.active_profile.lock() = Some(quantization);
        Ok(())
    }

    pub async fn stop(&self) -> Result<(), GenerationRuntimeError> {
        let child = self.child.lock().take();
        if let Some(mut child) = child {
            child.start_kill().map_err(GenerationRuntimeError::Stop)?;
            child.wait().await.map_err(GenerationRuntimeError::Stop)?;
            *self.active_profile.lock() = None;
            tracing::info!("stopped native generation runtime");
        }
        Ok(())
    }

    pub async fn switch_profile(
        &self,
        quantization: KreaQuantization,
        host: &str,
        port: u16,
    ) -> Result<(), GenerationRuntimeError> {
        if self.active_profile() == Some(quantization) && self.is_running() {
            return Ok(());
        }
        self.stop().await?;
        self.start(quantization, host, port)
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
                *self.active_profile.lock() = None;
                false
            }
            Err(error) => {
                tracing::warn!(%error, "could not inspect native generation runtime");
                false
            }
        }
    }

    pub fn active_profile(&self) -> Option<KreaQuantization> {
        *self.active_profile.lock()
    }

    pub fn executable(&self) -> &Path {
        &self.executable
    }

    /// Whether every artifact the profile needs exists under the model directory.
    pub fn has_profile_artifacts(&self, quantization: KreaQuantization) -> bool {
        let profile = krea_profile(quantization);
        let model_root = self.model_directory.join("krea-2");
        [profile.diffusion, profile.text_encoder, profile.vae]
            .iter()
            .all(|artifact| model_root.join(artifact.filename).is_file())
    }

    fn command(&self, profile: KreaProfile, model_root: &Path, host: &str, port: u16) -> Command {
        let mut command = Command::new(&self.executable);
        command
            .arg("--diffusion-model")
            .arg(model_root.join(profile.diffusion.filename))
            .arg("--llm")
            .arg(model_root.join(profile.text_encoder.filename))
            .arg("--vae")
            .arg(model_root.join(profile.vae.filename))
            .arg("--lora-model-dir")
            .arg(&self.lora_directory)
            .arg("--lora-apply-mode")
            .arg("at_runtime")
            .arg("--diffusion-fa")
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

    use super::{krea_profile, GenerationRuntime, KreaQuantization};

    #[test]
    fn profiles_share_the_quantized_text_encoder_and_vae() {
        let q2 = krea_profile(KreaQuantization::Q2);
        let q4 = krea_profile(KreaQuantization::Q4);

        assert_eq!(q2.text_encoder, q4.text_encoder);
        assert_eq!(q2.vae, q4.vae);
        assert!(q2.diffusion.size_bytes < q4.diffusion.size_bytes);
    }

    #[test]
    fn profile_artifacts_are_detected_on_disk() {
        let directory =
            std::env::temp_dir().join(format!("svs-artifacts-test-{}", uuid::Uuid::new_v4()));
        let model_root = directory.join("krea-2");
        let runtime = GenerationRuntime::new("/dev/null/sd-server", &directory, &directory);

        assert!(!runtime.has_profile_artifacts(KreaQuantization::Q2));

        let profile = krea_profile(KreaQuantization::Q2);
        fs::create_dir_all(&model_root).expect("model root should be created");
        for artifact in [profile.diffusion, profile.text_encoder, profile.vae] {
            fs::write(model_root.join(artifact.filename), b"x")
                .expect("artifact should be written");
        }

        assert!(runtime.has_profile_artifacts(KreaQuantization::Q2));
        assert!(!runtime.has_profile_artifacts(KreaQuantization::Q4));
        let _ = fs::remove_dir_all(&directory);
    }
}
