mod creative_runtime;
mod generation_runtime;
mod health;
mod model_store;

pub use creative_runtime::{CreativeRuntime, CreativeRuntimeError};
pub use generation_runtime::{
    krea_profile, GenerationRuntime, GenerationRuntimeError, KreaProfile, KreaQuantization,
    ModelArtifact,
};
pub use health::{check_all, HealthStatus, ServiceHealth, ServiceId};
pub use model_store::{DownloadProgress, ModelStore, ModelStoreError};

use std::{
    path::{Path, PathBuf},
    process::Stdio,
};

use parking_lot::Mutex;
use thiserror::Error;
use tokio::process::{Child, Command};

pub struct PythonRuntime {
    runtime_dir: PathBuf,
    generation_url: Option<String>,
    child: Mutex<Option<Child>>,
}

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("Python runtime is already running")]
    AlreadyRunning,
    #[error(
        "Python virtual environment executable not found at {0}; \
         create it with `uv sync --project python`"
    )]
    MissingPython(PathBuf),
    #[error("failed to resolve Python runtime directory {path}: {source}")]
    Resolve {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to start Python runtime: {0}")]
    Start(#[source] std::io::Error),
    #[error("failed while stopping Python runtime: {0}")]
    Stop(#[source] std::io::Error),
}

impl PythonRuntime {
    pub fn new(runtime_dir: impl Into<PathBuf>) -> Self {
        Self {
            runtime_dir: runtime_dir.into(),
            generation_url: None,
            child: Mutex::new(None),
        }
    }

    pub fn with_generation_url(mut self, url: impl Into<String>) -> Self {
        self.generation_url = Some(url.into());
        self
    }

    pub fn start(&self, host: &str, port: u16) -> Result<(), RuntimeError> {
        let mut slot = self.child.lock();
        if slot.is_some() {
            return Err(RuntimeError::AlreadyRunning);
        }

        // Resolve the runtime directory to an absolute path before spawning:
        // a relative executable plus a relative current_dir would be resolved
        // against the child's (already changed) working directory and fail with
        // ENOENT even though the venv exists.
        let runtime_dir =
            std::path::absolute(&self.runtime_dir).map_err(|source| RuntimeError::Resolve {
                path: self.runtime_dir.clone(),
                source,
            })?;
        let python = python_executable(&runtime_dir);
        if !python.exists() {
            return Err(RuntimeError::MissingPython(python));
        }

        let mut command = Command::new(&python);
        command
            .arg("main.py")
            .arg("--host")
            .arg(host)
            .arg("--port")
            .arg(port.to_string())
            .current_dir(&runtime_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        if let Some(url) = &self.generation_url {
            command.env("SVS_SD_CPP_URL", url);
        }
        let child = command.spawn().map_err(RuntimeError::Start)?;

        tracing::info!(pid = child.id(), "started Python vision runtime");
        *slot = Some(child);
        Ok(())
    }

    pub async fn stop(&self) -> Result<(), RuntimeError> {
        let child = self.child.lock().take();
        if let Some(mut child) = child {
            child.start_kill().map_err(RuntimeError::Stop)?;
            child.wait().await.map_err(RuntimeError::Stop)?;
            tracing::info!("stopped Python vision runtime");
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
                tracing::warn!(?status, "Python vision runtime exited");
                slot.take();
                false
            }
            Err(error) => {
                tracing::warn!(%error, "could not inspect Python vision runtime");
                false
            }
        }
    }
}

fn python_executable(runtime_dir: &Path) -> PathBuf {
    #[cfg(target_os = "windows")]
    return runtime_dir.join(".venv").join("Scripts").join("python.exe");

    #[cfg(not(target_os = "windows"))]
    runtime_dir.join(".venv").join("bin").join("python")
}

#[cfg(test)]
mod tests {
    use super::{python_executable, PythonRuntime};
    use std::path::PathBuf;

    #[test]
    fn start_supports_relative_runtime_directories() {
        let runtime_dir = PathBuf::from("python");
        if !python_executable(&runtime_dir).is_file() {
            eprintln!("skipping: python/.venv is not provisioned");
            return;
        }

        let tokio_runtime = tokio::runtime::Runtime::new().expect("tokio runtime should build");
        tokio_runtime.block_on(async move {
            // The configured path is relative (config/app.toml `paths.python_runtime`).
            // Regression: a relative executable plus a relative current_dir used to
            // resolve against the child's changed working directory and fail with ENOENT.
            let runtime = PythonRuntime::new(runtime_dir);
            runtime
                .start("127.0.0.1", 28765)
                .expect("start should resolve the relative runtime directory");
            assert!(runtime.is_running(), "spawned runtime should be alive");

            runtime
                .stop()
                .await
                .expect("stop should terminate the runtime");
        });
    }
}

// TODO: Add health-based readiness, restart policy, and graceful application shutdown wiring.
