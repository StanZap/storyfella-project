use std::{path::PathBuf, sync::Arc, time::Duration};

use thiserror::Error;
use uuid::Uuid;

use crate::{
    config::AppConfig,
    errors::{CodedError, ErrorCode},
    generation::{
        GenerationClient, GenerationClientError, GenerationJob, JobSubmission, ModelInfo,
    },
};

use super::{generation_profile, GenerationProfile, GenerationRuntime, GenerationRuntimeError};

/// The native server is a single resident process on loopback; the client's
/// base URL and these must agree (both come from `config/app.toml`).
const GENERATION_HOST: &str = "127.0.0.1";
const GENERATION_PORT: u16 = 7861;

#[derive(Clone)]
pub struct CreativeRuntime {
    client: GenerationClient,
    generation: Arc<GenerationRuntime>,
    asset_directory: PathBuf,
}

impl PartialEq for CreativeRuntime {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.generation, &other.generation)
    }
}

#[derive(Debug, Error)]
pub enum CreativeRuntimeError {
    #[error("could not start the generation server: {0}")]
    Generation(#[from] GenerationRuntimeError),
    #[error("generation server did not answer: {0}")]
    Client(#[from] GenerationClientError),
    #[error("generation server did not become ready within {0:?}")]
    ReadinessTimeout(Duration),
    #[error("generation job failed: {0}")]
    Job(String),
    #[error(
        "generation server on :7861 has {loaded} loaded; {requested} needs the server restarted. \
         Stop stale instances first (`svs runtime serve --force` or `pkill -f \"sd-server --diffusion\"`)"
    )]
    ProfileMismatch { loaded: String, requested: String },
    #[error("could not create generated asset directory {path}: {source}")]
    CreateAssetDirectory {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("could not import generated image from {path}: {source}")]
    ImportAsset {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl CodedError for CreativeRuntimeError {
    fn code(&self) -> ErrorCode {
        match self {
            Self::Generation(error) => error.code(),
            Self::Client(error) => error.code(),
            Self::ReadinessTimeout(_) => ErrorCode::RuntimeReadinessTimeout,
            Self::Job(_) => ErrorCode::RuntimeJobFailed,
            Self::ProfileMismatch { .. } => ErrorCode::RuntimeProfileMismatch,
            Self::CreateAssetDirectory { .. } => ErrorCode::RuntimeCreateAssetDirectory,
            Self::ImportAsset { .. } => ErrorCode::RuntimeImportAsset,
        }
    }
}

impl CreativeRuntime {
    pub fn new(config: &AppConfig) -> Self {
        Self {
            client: GenerationClient::new(&config.generation.base_url),
            generation: Arc::new(GenerationRuntime::new(
                &config.generation.executable,
                &config.model_dir,
                &config.generation.lora_dir,
            )),
            asset_directory: config.asset_dir.join("generated"),
        }
    }

    /// The native generation client (the Settings screen and the backend
    /// share the same server through it).
    pub fn client(&self) -> GenerationClient {
        self.client.clone()
    }

    pub fn profile(&self) -> GenerationProfile {
        generation_profile()
    }

    pub(crate) fn generation_runtime(&self) -> &GenerationRuntime {
        &self.generation
    }

    pub async fn ensure_ready(&self) -> Result<(), CreativeRuntimeError> {
        self.ensure_profile_ready().await
    }

    /// Ensures the native generation server is resident with the Qwen-Image-2.1
    /// profile, restarting only a server this process owns. A mismatched server
    /// started by another session cannot be restarted from here and is reported
    /// as [`CreativeRuntimeError::ProfileMismatch`].
    pub async fn ensure_profile_ready(&self) -> Result<(), CreativeRuntimeError> {
        let expected = generation_profile().diffusion.filename;

        if let Ok(capabilities) = self.client.capabilities().await {
            if model_matches(&capabilities.model, expected) {
                return Ok(());
            }
            if !self.generation.is_running() {
                return Err(CreativeRuntimeError::ProfileMismatch {
                    loaded: capabilities.model.name,
                    requested: expected.to_owned(),
                });
            }
            // Ours is resident but reports another model; restart it.
            self.generation.stop().await?;
        }

        self.generation
            .ensure_running(GENERATION_HOST, GENERATION_PORT)
            .await?;

        // Loading ~17.3 GiB of weights takes minutes; the cap must stay generous.
        let timeout = Duration::from_secs(600);
        let deadline = tokio::time::Instant::now() + timeout;
        let mut last_error = None;
        while tokio::time::Instant::now() < deadline {
            match self.client.capabilities().await {
                Ok(capabilities) if model_matches(&capabilities.model, expected) => return Ok(()),
                Ok(capabilities) => {
                    // A foreign server answered — ours could not bind.
                    return Err(CreativeRuntimeError::ProfileMismatch {
                        loaded: capabilities.model.name,
                        requested: expected.to_owned(),
                    });
                }
                Err(error) => last_error = Some(error),
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        if let Some(error) = last_error {
            tracing::warn!(%error, "generation server readiness timed out");
        }
        Err(CreativeRuntimeError::ReadinessTimeout(timeout))
    }

    /// Stops the resident generation server.
    pub async fn stop_generation_runtime(&self) -> Result<(), CreativeRuntimeError> {
        self.generation.stop().await?;
        Ok(())
    }

    /// Polls a submitted job to completion and stores its image in the asset
    /// directory, returning the stored path.
    pub async fn wait_for_job(
        &self,
        initial: JobSubmission,
    ) -> Result<PathBuf, CreativeRuntimeError> {
        if initial.status == "failed" {
            return Err(CreativeRuntimeError::Job(
                "the generation job was rejected on submission".to_owned(),
            ));
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(1200);
        while tokio::time::Instant::now() < deadline {
            let job = self.client.job(&initial.id).await?;
            match job.status.as_str() {
                "completed" => return self.store_image(&job).await,
                "failed" | "cancelled" => {
                    return Err(CreativeRuntimeError::Job(
                        job.error
                            .map(|error| error.message)
                            .unwrap_or_else(|| job.status.clone()),
                    ));
                }
                _ => tokio::time::sleep(Duration::from_millis(150)).await,
            }
        }
        Err(CreativeRuntimeError::Job(
            "generation exceeded twenty minutes".to_owned(),
        ))
    }

    /// Decodes a completed job's first image and writes it as a PNG in the
    /// generated-asset directory.
    async fn store_image(&self, job: &GenerationJob) -> Result<PathBuf, CreativeRuntimeError> {
        let bytes = job.first_image().map_err(CreativeRuntimeError::Client)?;
        tokio::fs::create_dir_all(&self.asset_directory)
            .await
            .map_err(|source| CreativeRuntimeError::CreateAssetDirectory {
                path: self.asset_directory.clone(),
                source,
            })?;
        let destination = self.asset_directory.join(format!("{}.png", Uuid::new_v4()));
        tokio::fs::write(&destination, &bytes)
            .await
            .map_err(|source| CreativeRuntimeError::ImportAsset {
                path: destination.clone(),
                source,
            })?;
        let absolute = tokio::fs::canonicalize(&destination)
            .await
            .map_err(|source| CreativeRuntimeError::ImportAsset {
                path: destination,
                source,
            })?;
        Ok(absolute)
    }

    pub async fn import_asset(&self, source: PathBuf) -> Result<String, CreativeRuntimeError> {
        // Already-stored finals (backend-generated images land here via the
        // job path) are returned as-is.
        if source.starts_with(&self.asset_directory) {
            return Ok(source.display().to_string());
        }
        tokio::fs::create_dir_all(&self.asset_directory)
            .await
            .map_err(|source| CreativeRuntimeError::CreateAssetDirectory {
                path: self.asset_directory.clone(),
                source,
            })?;
        let filename = format!("{}.png", Uuid::new_v4());
        let destination = self.asset_directory.join(filename);
        tokio::fs::copy(&source, &destination)
            .await
            .map_err(|error| CreativeRuntimeError::ImportAsset {
                path: source,
                source: error,
            })?;
        let absolute = tokio::fs::canonicalize(&destination)
            .await
            .map_err(|error| CreativeRuntimeError::ImportAsset {
                path: destination,
                source: error,
            })?;
        Ok(absolute.display().to_string())
    }
}

/// Whether the resident model is the profile's diffusion artifact. sd.cpp
/// reports the file name or its stem, so both are accepted.
fn model_matches(model: &ModelInfo, expected_filename: &str) -> bool {
    let expected_stem = expected_filename
        .strip_suffix(".safetensors")
        .unwrap_or(expected_filename);
    [model.name.as_str(), model.stem.as_str()]
        .iter()
        .any(|name| *name == expected_filename || *name == expected_stem)
}
