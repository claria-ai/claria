//! claria-desktop library root.
//!
//! Re-exports internal modules so that examples and integration tests
//! can exercise them directly (e.g. the bootstrap flow) without going
//! through the Tauri command layer.

pub mod audit;
pub mod aws;
pub mod config;
pub mod console;
pub mod http;
pub mod local_export;
pub mod logging;
pub mod report_authoring;
pub mod update;
