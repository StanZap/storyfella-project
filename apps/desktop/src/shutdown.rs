//! Process-exit cleanup for the native server this app starts.
//!
//! A native server outlives the process that spawned it unless something stops
//! it, and the app's exit paths are not uniformly graceful: the check-mode plan
//! run quits through `std::process::exit`, where no destructor runs and the
//! tokio `kill_on_drop` guard never fires. The workspace registers one bounded
//! stop here when it builds the backend; [`run`] calls it from the process's
//! final exits so a mid-generation quit does not orphan the server.

use std::sync::OnceLock;

/// The registered cleanup, run at each exit. A plain `Fn`, not a future: the
/// exit paths run after the async runtime is gone.
static ON_EXIT: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

/// Registers the process-exit cleanup. Called once, when the workspace builds
/// its backend; a later registration is ignored.
pub fn register(cleanup: impl Fn() + Send + Sync + 'static) {
    let _ = ON_EXIT.set(Box::new(cleanup));
}

/// Runs the registered cleanup, if any. Safe to call more than once — the
/// stop it performs owns nothing after the first call.
pub fn run() {
    if let Some(cleanup) = ON_EXIT.get() {
        cleanup();
    }
}
