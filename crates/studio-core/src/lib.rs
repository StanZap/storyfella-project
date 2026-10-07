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

pub mod authoring;
pub mod config;
pub mod errors;
pub mod llm;
pub mod paths;
pub mod persistence;
pub mod registry;
pub mod runtime;
pub mod story;
pub mod vision;
