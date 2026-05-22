//! Process Compose adapter.
//!
//! Three layers, intentionally separated so two of them are pure Rust and
//! testable without Tauri:
//!
//! * [`client`] — async REST client against PC's admin API.
//! * [`config`] — pure function: [`crate::registry::Registry`] → YAML string.
//! * [`lifecycle`] — owns the bundled sidecar via Tauri's shell plugin.
//!
//! See the Phase 0 spike report `claudedocs/spike-process-compose.md` for
//! the design decisions baked into each of these.

pub mod client;
pub mod config;
pub mod error;
pub mod lifecycle;
pub mod types;

pub use client::PcClient;
pub use config::to_yaml;
pub use error::{PcError, Result};
pub use lifecycle::{find_free_port, SidecarManager, DEFAULT_PORT};
pub use types::{LogsResponse, Process, ProjectStatus};
