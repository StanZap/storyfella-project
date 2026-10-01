//! Finding and stopping a managed server that a *different* process owns.
//!
//! A `sd-server`/`llama-server` started by one process — the desktop app, or
//! `svs runtime start` — is not reachable from another. So every start records
//! a small **marker**: the process id, the port, and the binary. A later process
//! reads the marker, checks the pid really is that binary (a recycled id must
//! not be killed), and stops it. A server with no marker (started by hand, or a
//! build that never recorded one) is reported as foreign and left alone.

use std::io;
use std::path::{Path, PathBuf};

/// What a running server recorded about itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerMarker {
    pub pid: u32,
    pub port: u16,
    pub executable: PathBuf,
}

/// Where the marker for a server listening on `port` lives.
pub fn marker_path(cache_dir: &Path, port: u16) -> PathBuf {
    cache_dir.join("servers").join(format!("{port}.marker"))
}

/// Where a detached server's stdout/stderr is logged.
pub fn server_log_path(cache_dir: &Path, port: u16) -> PathBuf {
    cache_dir.join("servers").join(format!("{port}.log"))
}

#[derive(Debug)]
pub enum MarkerError {
    Write { path: PathBuf, source: io::Error },
    Kill { pid: u32, source: io::Error },
}

impl std::fmt::Display for MarkerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Write { path, source } => {
                write!(f, "could not write the server marker {path:?}: {source}")
            }
            Self::Kill { pid, source } => write!(f, "could not stop process {pid}: {source}"),
        }
    }
}

impl std::error::Error for MarkerError {}

impl ServerMarker {
    /// Records this server so a later process can find it. Best-effort: a
    /// missing marker only costs the ability to stop an orphan later.
    pub fn write(&self, path: &Path) -> Result<(), MarkerError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| MarkerError::Write {
                path: path.to_path_buf(),
                source,
            })?;
        }
        let body = format!(
            "{}\n{}\n{}\n",
            self.pid,
            self.port,
            self.executable.display()
        );
        std::fs::write(path, body).map_err(|source| MarkerError::Write {
            path: path.to_path_buf(),
            source,
        })
    }

    /// Reads a marker, or `None` when it is missing or malformed.
    pub fn read(path: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        let mut lines = text.lines();
        let pid = lines.next()?.trim().parse().ok()?;
        let port = lines.next()?.trim().parse().ok()?;
        let executable = PathBuf::from(lines.next()?.trim());
        Some(Self {
            pid,
            port,
            executable,
        })
    }

    /// Whether the recorded pid is alive *and* is really this binary — the
    /// guard against killing an unrelated process that reused the id.
    pub fn is_live(&self) -> bool {
        pid_alive(self.pid) && process_runs(self.pid, &self.executable)
    }

    /// Stops the recorded server: `SIGTERM`, and `SIGKILL` if it does not go.
    pub fn kill(&self) -> Result<(), MarkerError> {
        if !self.is_live() {
            return Ok(());
        }
        signal(self.pid, "TERM")?;
        for _ in 0..20 {
            if !pid_alive(self.pid) {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        signal(self.pid, "KILL")
    }
}

/// Whether the process exists.
pub fn pid_alive(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        Path::new("/proc").join(pid.to_string()).exists()
    }
    #[cfg(not(target_os = "linux"))]
    {
        // No portable liveness check; assume it is (kill is guarded by the
        // binary match below, which is Linux-only, so be conservative).
        let _ = pid;
        false
    }
}

/// Whether `pid`'s command line names `executable`. Linux reads
/// `/proc/<pid>/cmdline`; elsewhere this cannot be verified, so it is `false`
/// and the marker is treated as not-ours (never killed by mistake).
fn process_runs(pid: u32, executable: &Path) -> bool {
    #[cfg(target_os = "linux")]
    {
        let Ok(raw) = std::fs::read(Path::new("/proc").join(pid.to_string()).join("cmdline"))
        else {
            return false;
        };
        let command = String::from_utf8_lossy(&raw).replace('\0', " ");
        let wanted = executable.to_string_lossy();
        command.contains(wanted.as_ref())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (pid, executable);
        false
    }
}

fn signal(pid: u32, signal: &str) -> Result<(), MarkerError> {
    std::process::Command::new("kill")
        .arg(format!("-{signal}"))
        .arg(pid.to_string())
        .status()
        .map_err(|source| MarkerError::Kill { pid, source })
        .and_then(|status| {
            if status.success() {
                Ok(())
            } else {
                Err(MarkerError::Kill {
                    pid,
                    source: io::Error::other(format!("kill -{signal} exited {status}")),
                })
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_path() -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("svs-servers-{}", uuid::Uuid::new_v4()));
        let path = marker_path(&dir, 7861);
        std::fs::create_dir_all(path.parent().expect("marker parent")).expect("temp dir");
        (dir, path)
    }

    #[test]
    fn a_marker_round_trips() {
        let (_dir, path) = temp_path();
        let marker = ServerMarker {
            pid: 4242,
            port: 7861,
            executable: PathBuf::from("/models/runtime/.../sd-server"),
        };
        marker.write(&path).expect("write");
        assert_eq!(ServerMarker::read(&path), Some(marker));
    }

    #[test]
    fn a_missing_or_malformed_marker_reads_as_none() {
        let (dir, path) = temp_path();
        assert_eq!(ServerMarker::read(&path), None);
        std::fs::write(&path, "not a marker").expect("write");
        assert_eq!(ServerMarker::read(&path), None);
        std::fs::write(marker_path(&dir, 7861), "12\n").expect("write");
        assert_eq!(ServerMarker::read(&path), None);
    }

    #[test]
    fn a_recycled_pid_is_not_considered_live() {
        // A pid that does not run the recorded binary (or does not exist at
        // all) is never killed.
        let dead = ServerMarker {
            pid: 4_000_000_000,
            port: 7861,
            executable: PathBuf::from("/nope/sd-server"),
        };
        assert!(!dead.is_live());
        assert!(dead.kill().is_ok());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_live_matching_process_is_recognised_and_killed() {
        let (_dir, path) = temp_path();
        // `sleep` is a stand-in server: it stays alive and has a cmdline.
        let exe = which_sleep();
        let mut child = std::process::Command::new(&exe)
            .arg("30")
            .spawn()
            .expect("spawn sleep");
        let marker = ServerMarker {
            pid: child.id(),
            port: 7861,
            executable: exe,
        };
        // Poll: right after `spawn` the child may not have `exec`d yet, so its
        // cmdline is briefly empty.
        let mut live = false;
        for _ in 0..40 {
            if marker.is_live() {
                live = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(
            live,
            "a live matching process should be recognised: exe={:?}",
            marker.executable
        );
        marker.write(&path).expect("write");
        marker.kill().expect("kill");
        let _ = child.wait();
        assert!(!marker.is_live(), "killed process is no longer live");
    }

    #[cfg(target_os = "linux")]
    fn which_sleep() -> PathBuf {
        for candidate in ["/bin/sleep", "/usr/bin/sleep"] {
            let path = PathBuf::from(candidate);
            if path.is_file() {
                return path;
            }
        }
        panic!("no `sleep` binary on this system");
    }
}
