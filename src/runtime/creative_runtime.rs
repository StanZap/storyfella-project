use std::{path::PathBuf, sync::Arc, time::Duration};

use thiserror::Error;
use uuid::Uuid;

use crate::{
    app::AppConfig,
    vision::{GenerationJobResponse, VisionClient, VisionClientError},
};

use super::{
    GenerationRuntime, GenerationRuntimeError, KreaQuantization, PythonRuntime, RuntimeError,
};

const VISION_BASE_URL: &str = "http://127.0.0.1:8765";

#[derive(Clone)]
pub struct CreativeRuntime {
    vision: VisionClient,
    python: Arc<PythonRuntime>,
    generation: Arc<GenerationRuntime>,
    profile: KreaQuantization,
    asset_directory: PathBuf,
}

impl PartialEq for CreativeRuntime {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.python, &other.python) && Arc::ptr_eq(&self.generation, &other.generation)
    }
}

#[derive(Debug, Error)]
pub enum CreativeRuntimeError {
    #[error("could not start the Krea runtime: {0}")]
    Generation(#[from] GenerationRuntimeError),
    #[error("could not start the vision service: {0}")]
    Python(#[from] RuntimeError),
    #[error("vision service did not become ready: {0}")]
    Vision(#[from] VisionClientError),
    #[error("vision service did not become ready within {0:?}")]
    ReadinessTimeout(Duration),
    #[error("generation job failed: {0}")]
    Job(String),
    #[error("generation job returned no image")]
    MissingImage,
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

impl CreativeRuntime {
    pub fn new(config: &AppConfig) -> Self {
        Self {
            vision: VisionClient::new(VISION_BASE_URL),
            python: Arc::new(
                PythonRuntime::new(&config.python_runtime)
                    .with_generation_url(&config.generation.base_url),
            ),
            generation: Arc::new(GenerationRuntime::new(
                &config.generation.executable,
                &config.model_dir,
                &config.generation.lora_dir,
            )),
            profile: config.generation.profile,
            asset_directory: config.asset_dir.join("generated"),
        }
    }

    pub fn vision_client(&self) -> VisionClient {
        self.vision.clone()
    }

    pub fn profile(&self) -> KreaQuantization {
        self.profile
    }

    pub async fn ensure_ready(&self) -> Result<(), CreativeRuntimeError> {
        if self.vision.health().await.is_ok() {
            if let Ok(capabilities) = self.vision.generation_capabilities().await {
                if capabilities.status == "ready" {
                    return Ok(());
                }
            }
        }

        if !self.generation.is_running() {
            self.generation.start(self.profile, "127.0.0.1", 7861)?;
        }
        if !self.python.is_running() {
            self.python.start("127.0.0.1", 8765)?;
        }

        let timeout = Duration::from_secs(90);
        let deadline = tokio::time::Instant::now() + timeout;
        let mut last_error = None;
        while tokio::time::Instant::now() < deadline {
            match self.vision.health().await {
                Ok(_) => match self.vision.generation_capabilities().await {
                    Ok(capabilities) if capabilities.status == "ready" => return Ok(()),
                    Ok(capabilities) => {
                        tracing::debug!(
                            error = capabilities.error,
                            "native generation runtime is still loading"
                        );
                    }
                    Err(error) => last_error = Some(error),
                },
                Err(error) => last_error = Some(error),
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        if let Some(error) = last_error {
            tracing::warn!(%error, "vision runtime readiness timed out");
        }
        Err(CreativeRuntimeError::ReadinessTimeout(timeout))
    }

    pub async fn wait_for_job(
        &self,
        initial: GenerationJobResponse,
    ) -> Result<PathBuf, CreativeRuntimeError> {
        if initial.status == "failed" {
            return Err(CreativeRuntimeError::Job(
                initial
                    .error
                    .unwrap_or_else(|| "generation failed".to_owned()),
            ));
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
        while tokio::time::Instant::now() < deadline {
            let job = self.vision.generation_job(&initial.id).await?;
            match job.status.as_str() {
                "completed" => {
                    return job
                        .image_path
                        .map(PathBuf::from)
                        .ok_or(CreativeRuntimeError::MissingImage);
                }
                "failed" | "cancelled" => {
                    return Err(CreativeRuntimeError::Job(
                        job.error.unwrap_or_else(|| job.status.clone()),
                    ));
                }
                _ => tokio::time::sleep(Duration::from_millis(150)).await,
            }
        }
        Err(CreativeRuntimeError::Job(
            "generation exceeded three minutes".to_owned(),
        ))
    }

    pub async fn import_asset(&self, source: PathBuf) -> Result<String, CreativeRuntimeError> {
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
