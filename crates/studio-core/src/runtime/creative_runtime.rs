use std::{path::PathBuf, sync::Arc, time::Duration};

use agentkit_ai::deployments::ImageDeployment;
use agentkit_ai::native::sd::{ClientError, JobSubmission, Residency};
use agentkit_ai::native::{control::Managed, service};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    config::{AppConfig, ConfigError},
    errors::{CodedError, ErrorCode},
};

/// How long a submitted job may run before the wait gives up.
const JOB_DEADLINE: Duration = Duration::from_secs(1200);

/// A snapshot of the selected deployment and where to store its output.
#[derive(Clone)]
pub struct CreativeRuntime {
    /// The exact image deployment captured for this runtime.
    native: Arc<Option<ImageDeployment>>,
    /// The state directory shared with the inference service.
    state_dir: PathBuf,
    asset_directory: PathBuf,
}

impl PartialEq for CreativeRuntime {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.native, &other.native)
    }
}

#[derive(Debug, Error)]
pub enum CreativeRuntimeError {
    #[error(transparent)]
    Configuration(#[from] ConfigError),
    #[error(transparent)]
    Service(#[from] service::ServiceError),
    #[error("generation server did not answer: {0}")]
    Client(#[from] ClientError),
    #[error("generation job failed: {0}")]
    Job(String),
    #[error("could not start or stop the chat server: {0}")]
    ChatServer(String),
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
            Self::Configuration(error) => error.code(),
            Self::Service(_) => ErrorCode::RuntimeServerExited,
            Self::Client(error) => error.code(),
            Self::Job(_) => ErrorCode::RuntimeJobFailed,
            Self::ChatServer(_) => ErrorCode::LlmServer,
            Self::CreateAssetDirectory { .. } => ErrorCode::RuntimeCreateAssetDirectory,
            Self::ImportAsset { .. } => ErrorCode::RuntimeImportAsset,
        }
    }
}

impl CreativeRuntime {
    pub fn new(config: &AppConfig) -> Self {
        Self::with_image(config, config.image.as_ref())
    }

    pub(crate) fn with_image(config: &AppConfig, image: Option<&ImageDeployment>) -> Self {
        Self {
            native: Arc::new(image.cloned()),
            state_dir: config.state_dir.clone(),
            asset_directory: config.asset_dir.join("generated"),
        }
    }

    fn native(&self) -> Result<&ImageDeployment, CreativeRuntimeError> {
        self.native
            .as_ref()
            .as_ref()
            .ok_or_else(|| ConfigError::NativeImageUnavailable.into())
    }

    fn managed(&self) -> Result<Managed, CreativeRuntimeError> {
        Ok(Managed::Image(self.native()?.clone()))
    }

    /// Changes the residency the next server start will use. Returns `false`
    /// for an unknown id, leaving the runtime alone.
    ///
    /// Residency is a launch-time flag, so a server already running keeps its
    /// old mode until it is stopped and started again; the caller decides
    /// whether to restart.
    pub fn set_residency(&mut self, residency: &str) -> bool {
        let Some(residency) = Residency::from_id(residency) else {
            return false;
        };
        let Some(native) = Arc::make_mut(&mut self.native).as_mut() else {
            return false;
        };
        native.residency = residency;
        true
    }

    /// Whether every artifact the served profile loads exists on disk.
    pub fn has_artifacts(&self) -> bool {
        self.native().is_ok_and(|native| {
            native
                .artifact_plan()
                .iter()
                .all(|plan| plan.path.is_file())
        })
    }

    /// The model file name the resident server should report; a server holding
    /// the port but serving something else reports a different one.
    pub fn expected_model(&self) -> Result<String, CreativeRuntimeError> {
        let image = &self.native()?;
        Ok(image
            .artifact_plan()
            .first()
            .and_then(|plan| plan.path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| image.profile().diffusion.filename.to_owned()))
    }

    fn service(&self) -> service::Client {
        service::Client::new(&self.state_dir)
    }

    pub async fn is_running(&self) -> bool {
        let Ok(id) = self
            .managed()
            .and_then(|model| Ok(service::model_id(&model)?))
        else {
            return false;
        };
        self.service()
            .status()
            .await
            .is_ok_and(|status| status.resident.iter().any(|worker| worker.id == id))
    }

    pub async fn ensure_ready(&self) -> Result<(), CreativeRuntimeError> {
        Ok(self.service().warm(&self.managed()?).await?)
    }

    pub async fn acquire_image(&self) -> Result<service::Lease, CreativeRuntimeError> {
        Ok(self.service().image(self.native()?).await?)
    }

    pub async fn stop_generation_runtime(&self) -> Result<(), CreativeRuntimeError> {
        Ok(self.service().unload(&self.managed()?).await?)
    }

    /// Waits for a submitted job to complete and stores its image in the asset
    /// directory, returning the stored path.
    pub async fn wait_for_job(
        &self,
        lease: service::Lease,
        initial: JobSubmission,
    ) -> Result<PathBuf, CreativeRuntimeError> {
        if initial.status == "failed" {
            lease.finish().await?;
            return Err(CreativeRuntimeError::Job(
                "the generation job was rejected on submission".to_owned(),
            ));
        }
        let bytes = lease.image_client().wait(&initial.id, JOB_DEADLINE).await;
        if bytes.is_ok() || matches!(bytes, Err(ClientError::JobFailed(_))) {
            lease.finish().await?;
        }
        let bytes = bytes.map_err(|error| match error {
            ClientError::JobFailed(message) => CreativeRuntimeError::Job(message),
            ClientError::WaitTimeout(_) => {
                CreativeRuntimeError::Job("generation exceeded twenty minutes".to_owned())
            }
            other => CreativeRuntimeError::Client(other),
        })?;
        self.store_image(&bytes).await
    }

    /// Writes a completed job's PNG bytes into the generated-asset directory.
    pub(crate) async fn store_image(&self, bytes: &[u8]) -> Result<PathBuf, CreativeRuntimeError> {
        tokio::fs::create_dir_all(&self.asset_directory)
            .await
            .map_err(|source| CreativeRuntimeError::CreateAssetDirectory {
                path: self.asset_directory.clone(),
                source,
            })?;
        let destination = self.asset_directory.join(format!("{}.png", Uuid::new_v4()));
        tokio::fs::write(&destination, bytes)
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
