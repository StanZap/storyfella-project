//! Storyfella Studio: the GPUI window and its views, the app state they
//! share, the command language every surface funnels through, and the plan
//! driver that plays YAML plans through those same commands.
//!
//! The domain layer lives in the `storyfella-studio-core` crate and is named explicitly
//! (`storyfella_studio_core::…`) rather than re-exported here, so the boundary
//! between the app and the domain stays visible.

pub mod commands;
pub mod driver;
pub mod host;
#[cfg(unix)]
mod live_mcp;
pub mod preview;
pub mod recents;
pub mod sample;
pub mod state;
pub mod status;
pub mod ui;
pub mod writing;
