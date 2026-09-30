//! `cargo xtask` — the workspace's own task runner.
//!
//! Every subcommand wraps a workflow documented in `docs/development.md`, so
//! the exact `dx` / `cargo` / `npm` / `python` invocations live in one
//! reviewable place instead of being retyped from memory. Run it through the
//! alias in `.cargo/config.toml`:
//!
//! ```text
//! cargo xtask run          # build (debug) and launch the desktop app
//! cargo xtask run -r       # …in release
//! cargo xtask bundle       # release AppImage into dist/
//! ```
//!
//! The debug build is the default everywhere; `-r`/`--release` opts into the
//! release profile. Commands that launch a process propagate its exit code, so
//! `cargo xtask plan … --mode check` can gate a script.

mod cli;
mod proc;
mod tasks;
mod workspace;

use std::process::ExitCode;

use clap::Parser;

fn main() -> ExitCode {
    match tasks::dispatch(cli::Cli::parse()) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("\nxtask: {error:#}");
            ExitCode::FAILURE
        }
    }
}
