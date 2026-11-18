//! Storyfella Studio's domain layer.
//!
//! Everything the application and the CLI share, with no dependency on the GUI
//! toolkit: the artifact registry and typed operation/pipeline layer
//! ([`registry`]), SQLite persistence ([`persistence`]), the LLM
//! client ([`llm`]), the runtime adapter over the shared native-inference crate
//! ([`runtime`]), the error-code
//! registry ([`errors`]) and the project-root resolution every path hangs off
//! ([`paths`]).
//!
//! The desktop application re-exports none of this: it names `storyfella_studio_core::…`
//! explicitly, so the direction of the dependency stays visible at every call
//! site.

pub mod artwork;
pub mod atelier;
pub mod authoring;
pub mod catalog;
pub mod config;
#[cfg(unix)]
pub mod desktop_bridge;
pub mod errors;
mod identity;
pub mod llm;
pub mod paths;
pub mod persistence;
pub mod planner;
pub mod playback;
pub mod project;
pub mod registry;
pub mod runtime;
pub mod story;
pub mod vision;
pub mod workflows;

pub mod ai_config;
