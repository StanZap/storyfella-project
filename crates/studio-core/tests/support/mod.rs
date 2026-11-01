use agentkit_ai::native::control;
use std::path::PathBuf;

/// Stops only an owned acceptance server, including when a test panics.
pub struct ServerCleanup {
    pub state: PathBuf,
    pub port: u16,
}

impl Drop for ServerCleanup {
    fn drop(&mut self) {
        if let Err(error) = control::stop(&self.state, self.port) {
            eprintln!("Acceptance server cleanup failed: {error}");
        }
    }
}
