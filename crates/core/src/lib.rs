//! Smart Visual Sequencer's domain layer.
//!
//! Everything the application and the CLI share, with no dependency on the GUI
//! toolkit: the artifact registry and typed operation/pipeline layer
//! ([`registry`]), SQLite persistence ([`persistence`]), the native
//! stable-diffusion.cpp generation client ([`generation`]), the LM Studio
//! client ([`llm`]), the process supervision and model store ([`runtime`]),
//! the asset-catalog boundary ([`assets`]), the error-code registry
//! ([`errors`]) and the project-root resolution every path hangs off
//! ([`paths`]).
//!
//! The desktop application re-exports none of this: it names `svs_core::…`
//! explicitly, so the direction of the dependency stays visible at every call
//! site.

pub mod assets;
pub mod config;
pub mod errors;
pub mod generation;
pub mod llm;
pub mod paths;
pub mod persistence;
pub mod registry;
pub mod runtime;
