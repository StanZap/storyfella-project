use std::{path::PathBuf, sync::Arc, time::Duration};

use agentkit_ai::deployments::ImageDeployment;
use agentkit_ai::native::control::{self, ControlError, Managed, ServerState};
use agentkit_ai::native::sd::{Client, ClientError, JobSubmission, Residency};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    config::{AppConfig, ConfigError},
    errors::{CodedError, ErrorCode},
};

/// How long a submitted job may run before the wait gives up.
const JOB_DEADLINE: Duration = Duration::from_secs(1200);

/// The app's handle on the shared native-inference runtime: the machine's
/// `sd-server`, a `sdcpp` client for it, and the asset directory generated
/// images are stored in.
///
/// The process itself is owned by the **shared lifecycle owner** — the
/// `agentkit_ai::native::control` helpers over one shared state directory — not
/// by this object. Storyfella, Atelier and the Clipforge apps all drive that
/// owner, so a server one front end started is reused by the others instead of
/// a second process being launched on the same port. Starting a server
/// therefore writes a marker this app can see from a later process (see
/// [`agentkit_ai::native::servers`]), and it is detached: it outlives the app.
#[derive(Clone)]
pub struct CreativeRuntime {
    /// The deployment this runtime serves: its executable, address, weights,
    /// and residency. Kept whole so lifecycle actions pass the shared owner the
    /// exact server to ensure (see [`CreativeRuntime::set_residency`]).
    native: Arc<Option<NativeImage>>,
    /// The shared state directory the lifecycle owner records markers and logs
    /// in (see [`crate::paths::state_dir`]).
    state_dir: PathBuf,
    asset_directory: PathBuf,
}

#[derive(Clone)]
struct NativeImage {
    client: Client,
    deployment: ImageDeployment,
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
    Control(#[from] ControlError),
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
            Self::Control(error) => error.code(),
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
        Self {
            native: Arc::new(config.image.as_ref().map(|image| NativeImage {
                client: Client::new(format!("http://{}:{}", image.host, image.port)),
                deployment: image.clone(),
            })),
            state_dir: config.state_dir.clone(),
            asset_directory: config.asset_dir.join("generated"),
        }
    }

    /// The deployment as the shared lifecycle owner sees it.
    fn native(&self) -> Result<&NativeImage, CreativeRuntimeError> {
        self.native
            .as_ref()
            .as_ref()
            .ok_or_else(|| ConfigError::NativeImageUnavailable.into())
    }

    fn managed(&self) -> Result<Managed, CreativeRuntimeError> {
        Ok(Managed::Image(self.native()?.deployment.clone()))
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
        native.deployment.residency = residency;
        true
    }

    /// The native generation client (the Settings screen and the backend
    /// share the same server through it).
    pub fn client(&self) -> Result<Client, CreativeRuntimeError> {
        Ok(self.native()?.client.clone())
    }

    /// Whether every artifact the served profile loads exists on disk.
    pub fn has_artifacts(&self) -> bool {
        self.native().is_ok_and(|native| {
            native
                .deployment
                .artifact_plan()
                .iter()
                .all(|plan| plan.path.is_file())
        })
    }

    /// The model file name the resident server should report; a server holding
    /// the port but serving something else reports a different one.
    pub fn expected_model(&self) -> Result<String, CreativeRuntimeError> {
        let image = &self.native()?.deployment;
        Ok(image
            .artifact_plan()
            .first()
            .and_then(|plan| plan.path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| image.profile().diffusion.filename.to_owned()))
    }

    /// Whether the shared native server is answering on the deployment's
    /// address. Read-only: nothing is started or stopped.
    pub async fn is_running(&self) -> bool {
        let Ok(managed) = self.managed() else {
            return false;
        };
        matches!(
            control::status(&managed, &self.state_dir).await,
            ServerState::Up { .. }
        )
    }

    /// Ensures the configured model is the one resident on the deployment's
    /// address, starting a server when none is up. A server the shared owner
    /// recorded but that holds a different model is replaced; a server started
    /// outside the shared owner is left alone and reported as a
    /// [`ControlError::Image`] profile mismatch.
    pub async fn ensure_ready(&self) -> Result<(), CreativeRuntimeError> {
        control::load(&self.managed()?, &self.state_dir).await?;
        Ok(())
    }

    /// Stops the resident generation server the shared owner recorded. A server
    /// started outside the shared owner has no marker and is left alone.
    pub async fn stop_generation_runtime(&self) -> Result<(), CreativeRuntimeError> {
        control::stop(&self.state_dir, self.native()?.deployment.port)?;
        Ok(())
    }

    /// Waits for a submitted job to complete and stores its image in the asset
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
        let bytes = self
            .native()?
            .client
            .wait(&initial.id, JOB_DEADLINE)
            .await
            .map_err(|error| match error {
                ClientError::JobFailed(message) => CreativeRuntimeError::Job(message),
                ClientError::WaitTimeout(_) => {
                    CreativeRuntimeError::Job("generation exceeded twenty minutes".to_owned())
                }
                other => CreativeRuntimeError::Client(other),
            })?;
        self.store_image(&bytes).await
    }

    /// Writes a completed job's PNG bytes into the generated-asset directory.
    async fn store_image(&self, bytes: &[u8]) -> Result<PathBuf, CreativeRuntimeError> {
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
