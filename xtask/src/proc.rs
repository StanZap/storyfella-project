//! Running child processes: one place that logs what is about to run, streams
//! its output, and turns a non-zero exit into an error with the command in it.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};

use anyhow::{bail, Context, Result};

/// Progress lines go to stderr so a task's own stdout stays useful in a pipe.
pub fn step(message: impl AsRef<str>) {
    eprintln!("==> {}", message.as_ref());
}

/// A successful completion line.
pub fn done(message: impl AsRef<str>) {
    eprintln!("    {}", message.as_ref());
}

/// Render a command the way a user would type it.
pub fn display(cmd: &Command) -> String {
    let mut parts = vec![cmd.get_program().to_string_lossy().into_owned()];
    parts.extend(cmd.get_args().map(|arg| arg.to_string_lossy().into_owned()));
    parts.join(" ")
}

/// Run a prepared command to completion, inheriting stdio, and fail if it does
/// not exit successfully.
pub fn require(cmd: &mut Command) -> Result<()> {
    let shown = display(cmd);
    step(&shown);
    let status = cmd
        .status()
        .with_context(|| format!("failed to spawn `{shown}`"))?;
    if !status.success() {
        bail!("`{shown}` exited with {}", describe(status));
    }
    Ok(())
}

/// Run a prepared command to completion, returning its status without treating
/// a non-zero exit as an error (used where the child's code is the result).
pub fn status(cmd: &mut Command) -> Result<ExitStatus> {
    let shown = display(cmd);
    step(&shown);
    cmd.status()
        .with_context(|| format!("failed to spawn `{shown}`"))
}

/// Like [`status`], but when `log` is set the child's stdout/stderr are streamed
/// *and* appended to that file.
pub fn launch(cmd: &mut Command, log: Option<&Path>) -> Result<ExitStatus> {
    let Some(log) = log else {
        return status(cmd);
    };

    let shown = display(cmd);
    step(&shown);
    if let Some(parent) = log.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let file = std::fs::File::create(log)
        .with_context(|| format!("opening {} for writing", log.display()))?;
    let sink = Arc::new(Mutex::new(file));

    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .with_context(|| format!("failed to spawn `{shown}`"))?;

    let stdout = child.stdout.take().expect("stdout was piped");
    let stderr = child.stderr.take().expect("stderr was piped");
    let out_tee = tee(stdout, Arc::clone(&sink), false);
    let err_tee = tee(stderr, sink, true);

    let status = child.wait().context("waiting for the child process")?;
    // The readers see EOF when the child exits; joining keeps the log complete.
    let _ = out_tee.join();
    let _ = err_tee.join();
    Ok(status)
}

/// Copy `reader` to the console and to `sink` line by line, byte-for-byte,
/// until EOF.
fn tee<R>(
    reader: R,
    sink: Arc<Mutex<std::fs::File>>,
    to_stderr: bool,
) -> std::thread::JoinHandle<()>
where
    R: Read + Send + 'static,
{
    std::thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        let mut line = Vec::new();
        loop {
            line.clear();
            match reader.read_until(b'\n', &mut line) {
                Ok(0) => break,
                Ok(_) => {
                    if to_stderr {
                        let mut console = std::io::stderr();
                        let _ = console.write_all(&line);
                        let _ = console.flush();
                    } else {
                        let mut console = std::io::stdout();
                        let _ = console.write_all(&line);
                        let _ = console.flush();
                    }
                    if let Ok(mut file) = sink.lock() {
                        let _ = file.write_all(&line);
                    }
                }
                Err(_) => break,
            }
        }
    })
}

/// `exit code N` or `a signal`, for error messages.
pub fn describe(status: ExitStatus) -> String {
    match status.code() {
        Some(code) => format!("exit code {code}"),
        None => "a signal".to_string(),
    }
}

/// Run `<program> <version_arg>` and return its first stdout line, if it works.
pub fn version(program: &str, version_arg: &str) -> Option<String> {
    let output = Command::new(program).arg(version_arg).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text.lines().next().unwrap_or("").trim();
    Some(if line.is_empty() {
        "found".to_string()
    } else {
        line.to_string()
    })
}
