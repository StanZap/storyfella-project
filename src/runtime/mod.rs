mod model_store;

pub use model_store::{ModelStore, ModelStoreError};

use std::{
    path::{Path, PathBuf},
    process::Stdio,
};

use parking_lot::Mutex;
use thiserror::Error;
use tokio::process::{Child, Command};

pub struct PythonRuntime {
    runtime_dir: PathBuf,
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
            child: Mutex::new(None),
        }
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

        let child = Command::new(&python)
            .arg("main.py")
            .arg("--host")
            .arg(host)
            .arg("--port")
            .arg(port.to_string())
            .current_dir(&self.runtime_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .map_err(RuntimeError::Start)?;

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
        self.child.lock().is_some()
    }
}

fn python_executable(runtime_dir: &Path) -> PathBuf {
    #[cfg(target_os = "windows")]
    return runtime_dir.join(".venv").join("Scripts").join("python.exe");

    #[cfg(not(target_os = "windows"))]
    runtime_dir.join(".venv").join("bin").join("python")
}

// TODO: Add health-based readiness, restart policy, and graceful application shutdown wiring.
