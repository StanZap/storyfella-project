mod creative_runtime;
mod generation_runtime;
mod model_store;

pub use creative_runtime::{CreativeRuntime, CreativeRuntimeError};
pub use generation_runtime::{
    krea_profile, GenerationRuntime, GenerationRuntimeError, KreaProfile, KreaQuantization,
    ModelArtifact,
};
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
    #[error("Python virtual environment executable not found at {0}")]
    MissingPython(PathBuf),
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

        let python = python_executable(&self.runtime_dir);
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
            .current_dir(&self.runtime_dir)
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

// TODO: Add health-based readiness, restart policy, and graceful application shutdown wiring.
