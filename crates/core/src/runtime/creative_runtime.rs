use std::{path::PathBuf, sync::Arc, time::Duration};

use clipforge_inference::sd::{
    Client, ClientError, JobSubmission, Residency, Runtime, RuntimeError,
};
use clipforge_inference::ImageDeployment;
use thiserror::Error;
use uuid::Uuid;

use crate::{
    config::AppConfig,
    errors::{CodedError, ErrorCode},
};

use clipforge_inference::servers::{marker_path, server_log_path, ServerMarker};

/// How long a submitted job may run before the wait gives up.
const JOB_DEADLINE: Duration = Duration::from_secs(1200);

/// The app's handle on the shared native-inference runtime: the resident
/// `sd-server` process, an `sdcpp` client for it, and the asset directory
/// generated images are stored in.
#[derive(Clone)]
pub struct CreativeRuntime {
    client: Client,
    /// The deployment this runtime serves: its executable, address, weights,
    /// and residency. Kept whole so the runtime can be rebuilt when the
    /// residency changes (see [`CreativeRuntime::set_residency`]).
    image: ImageDeployment,
    runtime: Arc<Runtime>,
    asset_directory: PathBuf,
    /// Where the process marker is written, so a later process can stop a
    /// leftover server this one leaves behind (see [`super::servers`]).
    cache_dir: PathBuf,
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
        let image = config.image.clone();
        Self {
            client: Client::new(format!("http://{}:{}", image.host, image.port)),
            runtime: Arc::new(image.runtime()),
            asset_directory: config.asset_dir.join("generated"),
            cache_dir: config.cache_dir.clone(),
            image,
        }
    }

    /// A runtime whose server is **detached**: it logs to the cache directory
    /// and is not killed when this process exits, so `svs runtime start` leaves
    /// a resident server behind.
    pub fn detached(config: &AppConfig) -> Self {
        let image = config.image.clone();
        let runtime = image
            .runtime()
            .keep_alive(server_log_path(&config.cache_dir, image.port));
        Self {
            client: Client::new(format!("http://{}:{}", image.host, image.port)),
            runtime: Arc::new(runtime),
            asset_directory: config.asset_dir.join("generated"),
            cache_dir: config.cache_dir.clone(),
            image,
        }
    }

    /// The residency the runtime starts the server with (see
    /// [`Residency`]).
    pub fn residency(&self) -> &'static str {
        self.image.residency.as_str()
    }

    /// Changes the residency the next server start will use, rebuilding the
    /// runtime. Returns `false` for an unknown id, leaving the runtime alone.
    ///
    /// Residency is a launch-time flag, so a server already running keeps its
    /// old mode until it is stopped and started again; the caller decides
    /// whether to restart.
    pub fn set_residency(&mut self, residency: &str) -> bool {
        let Some(residency) = Residency::from_id(residency) else {
            return false;
        };
        self.image.residency = residency;
        self.runtime = Arc::new(self.image.runtime());
        true
    }

    /// The pid of the server this process started, when it owns one.
    pub fn pid(&self) -> Option<u32> {
        self.runtime.pid()
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
            .ensure_ready(&self.image.host, self.image.port)
            .await
            .map_err(|error| match error {
                // The shared runtime's profile mismatch carries the loaded and
                // requested model names; carry them into the app's own wording.
                RuntimeError::ProfileMismatch {
                    loaded, requested, ..
                } => CreativeRuntimeError::ProfileMismatch {
                    address: format!("{}:{}", self.image.host, self.image.port),
                    loaded,
                    requested,
                },
                RuntimeError::ReadinessTimeout(timeout) => {
                    CreativeRuntimeError::ReadinessTimeout(timeout)
                }
                other => CreativeRuntimeError::Runtime(other),
            })?;
        self.record_marker();
        Ok(())
    }

    /// Stops the resident generation server this process owns.
    pub async fn stop_generation_runtime(&self) -> Result<(), CreativeRuntimeError> {
        self.runtime.stop().await?;
        self.clear_marker();
        Ok(())
    }

    /// Restarts the resident generation server (stop, then start). A server
    /// this process does not own cannot be restarted — stopping it is a no-op,
    /// and the restart fails with the profile mismatch.
    pub async fn restart_generation_runtime(&self) -> Result<(), CreativeRuntimeError> {
        self.stop_generation_runtime().await?;
        self.ensure_profile_ready().await
    }

    /// Stops the resident generation server this process owns without an async
    /// runtime, waiting up to `timeout` for it to exit, and clears its marker.
    ///
    /// For the app's exit paths, which run outside the async runtime — and, for
    /// a check-mode run, through `std::process::exit`, where destructors never
    /// run and `kill_on_drop` never fires, so the server would be orphaned.
    /// Returns `false` when the server did not exit within `timeout`.
    pub fn stop_generation_runtime_blocking(&self, timeout: Duration) -> bool {
        let stopped = self.runtime.stop_blocking(timeout);
        self.clear_marker();
        stopped
    }

    /// Records the running server's pid so a *later* process (a fresh CLI, the
    /// app after a crash) can stop an orphan. Best-effort: a marker that cannot
    /// be written only costs that later cleanup.
    fn record_marker(&self) {
        if let Some(pid) = self.runtime.pid() {
            let marker = ServerMarker {
                pid,
                port: self.image.port,
                executable: self.image.executable.clone(),
            };
            let _ = marker.write(&marker_path(&self.cache_dir, self.image.port));
        }
    }

    fn clear_marker(&self) {
        let _ = std::fs::remove_file(marker_path(&self.cache_dir, self.image.port));
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
