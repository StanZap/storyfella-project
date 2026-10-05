//! The runtime adapter over the shared native-inference crate.
//!
//! The served profile, the resident `sd-server` process, the model store, and
//! the `sdcpp` HTTP client all live in `clipforge_inference` (shared with the
//! sibling `clipforge` workspace). This module is the storyfella-specific
//! layer over them: [`CreativeRuntime`] binds the shared runtime and client to
//! the app's [`AppConfig`](crate::config::AppConfig) and hides the job
//! bookkeeping (submitting, waiting, and storing the resulting PNG in the
//! asset directory), and the `health` module probes the services the app
//! depends on.

mod creative_runtime;
mod health;
mod provision;

pub use clipforge_inference::servers::{kill_servers, marker_path, server_log_path, ServerMarker};
pub use creative_runtime::{CreativeRuntime, CreativeRuntimeError};
pub use health::{check_all, HealthStatus, ServiceHealth, ServiceId};
pub use provision::{image_artifacts, provision_image_model, ModelArtifactStatus};
