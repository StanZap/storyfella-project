use std::{path::PathBuf, sync::Arc, time::Duration};

use clipforge_inference::sd::{Client, ClientError, JobSubmission, Runtime, RuntimeError};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    config::AppConfig,
    errors::{CodedError, ErrorCode},
};

/// How long a submitted job may run before the wait gives up.
const JOB_DEADLINE: Duration = Duration::from_secs(1200);

/// The app's handle on the shared native-inference runtime: the resident
/// `sd-server` process, an `sdcpp` client for it, and the asset directory
/// generated images are stored in.
#[derive(Clone)]
pub struct CreativeRuntime {
    client: Client,
    runtime: Arc<Runtime>,
    asset_directory: PathBuf,
    /// The deployment's listen address — readiness and the mismatch message
    /// both name it.
    host: String,
    port: u16,
}

impl PartialEq for CreativeRuntime {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.runtime, &other.runtime)
    }
}

#[derive(Debug, Error)]
pub enum CreativeRuntimeError {
    #[error("could not start the generation server: {0}")]
    Runtime(#[from] RuntimeError),
    #[error("generation server did not answer: {0}")]
    Client(#[from] ClientError),
    #[error("generation server did not become ready within {0:?}")]
    ReadinessTimeout(Duration),
    #[error("generation job failed: {0}")]
    Job(String),
    #[error(
        "generation server on {address} has {loaded} loaded; {requested} needs the server restarted. \
         Stop stale instances first (`svs runtime serve --force` or `pkill -f \"sd-server --diffusion\"`)"
    )]
    ProfileMismatch {
        address: String,
        loaded: String,
        requested: String,
    },
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
            Self::Runtime(error) => error.code(),
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
        let image = &config.image;
        Self {
            client: Client::new(format!("http://{}:{}", image.host, image.port)),
            runtime: Arc::new(image.runtime()),
            asset_directory: config.asset_dir.join("generated"),
            host: image.host.clone(),
            port: image.port,
        }
    }

    /// The native generation client (the Settings screen and the backend
    /// share the same server through it).
    pub fn client(&self) -> Client {
        self.client.clone()
    }

    /// Whether every artifact the served profile loads exists on disk.
    pub fn has_artifacts(&self) -> bool {
        self.runtime.has_artifacts()
    }

    /// Whether the resident server this app started is still alive.
    pub fn is_running(&self) -> bool {
        self.runtime.is_running()
    }

    pub async fn ensure_ready(&self) -> Result<(), CreativeRuntimeError> {
        self.ensure_profile_ready().await
    }

    /// Ensures the native generation server is resident with the Qwen-Image-2.1
    /// profile, restarting only a server this process owns. A mismatched server
    /// started by another session cannot be restarted from here and is reported
    /// as [`CreativeRuntimeError::ProfileMismatch`].
    pub async fn ensure_profile_ready(&self) -> Result<(), CreativeRuntimeError> {
        self.runtime
            .ensure_ready(&self.host, self.port)
            .await
            .map_err(|error| match error {
                // The shared runtime's profile mismatch carries the loaded and
                // requested model names; carry them into the app's own wording.
                RuntimeError::ProfileMismatch {
                    loaded, requested, ..
                } => CreativeRuntimeError::ProfileMismatch {
                    address: format!("{}:{}", self.host, self.port),
                    loaded,
                    requested,
                },
                RuntimeError::ReadinessTimeout(timeout) => {
                    CreativeRuntimeError::ReadinessTimeout(timeout)
                }
                other => CreativeRuntimeError::Runtime(other),
            })
    }

    /// Stops the resident generation server.
    pub async fn stop_generation_runtime(&self) -> Result<(), CreativeRuntimeError> {
        self.runtime.stop().await?;
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
        let bytes =
            self.client
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
