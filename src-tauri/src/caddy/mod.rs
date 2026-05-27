//! Caddy adapter.
//!
//! Parallel structure to [`crate::process_compose`]:
//!
//! * [`client`] — async REST client against Caddy's admin API.
//! * [`config`] — pure function: `Registry` → `CaddyConfig` document.
//! * [`lifecycle`] — owns the bundled `caddy` sidecar via Tauri.
//!
//! Spike findings baked in (see `claudedocs/spike-caddy.md`):
//!
//! * `automatic_https.disable_redirects = true` and `apps.http.http_port = 0`
//!   to keep Caddy off `:80` (otherwise it silently collides with anything
//!   else holding the port).
//! * `caddy run --config ...` is the launch shape — not `caddy start`,
//!   which has hanging-shell behaviour.
//! * `@id` stamped on every route so DELETE/PATCH are one-call ops.

pub mod client;
pub mod config;
pub mod error;
pub mod lifecycle;
pub mod types;

pub use client::CaddyClient;
pub use config::{
    bootstrap_config, build_config, build_config_filtered, project_to_route, with_access_log,
    CaddyPorts, CertPaths, ACCESS_LOGGER, ACCESS_LOG_FILE,
};
pub use error::{CaddyError, Result};
pub use lifecycle::{
    find_free_https_port, find_free_port, CaddySidecar, ADMIN_SCAN_RANGE, DEFAULT_ADMIN_PORT,
    DEFAULT_HTTPS_PORT,
};
pub use types::{
    AdminConfig, AppsConfig, AutomaticHttps, CaddyConfig, HttpApp, MatchClause, Route, Server,
    TlsApp, TlsCertFile, TlsCertificates,
};
