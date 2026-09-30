//! The Dioxus desktop application: the UI surfaces, the app state they share,
//! the command language every surface funnels through, and the plan driver that
//! plays YAML plans through those same commands.
//!
//! The domain layer lives in the `svs-core` crate and is named explicitly
//! (`svs_core::registry::…`) rather than re-exported here, so the boundary
//! between the app and the domain stays visible.

pub mod app;
pub mod commands;
pub mod driver;
pub mod state;
pub mod ui;
