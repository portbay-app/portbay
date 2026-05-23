//! dnsmasq adapter — wildcard DNS for `*.<suffix>` → `127.0.0.1`.
//!
//! One bundled sidecar (or PATH-found binary) per app instance. The
//! daemon runs on a non-privileged loopback port; macOS routes
//! `.<suffix>` queries to it via `/etc/resolver/<suffix>`, which is
//! written by a separate sudo-driven command (the resolver-install
//! card on the backlog).
//!
//! Until that resolver file is in place, dnsmasq is harmless — it
//! answers loopback queries that nobody sends. The lifecycle work
//! lands first so the install flow has a running daemon to point at.

pub mod config;
pub mod error;
pub mod lifecycle;
pub mod resolver;

pub use config::{build_config, default_config_path, write_config};
pub use error::{DnsmasqError, Result};
pub use lifecycle::{
    binary_available, find_free_port, DnsmasqSidecar, DEFAULT_PORT, PORT_SCAN_RANGE,
};
pub use resolver::{
    install_via_osascript, is_installed, read_installed, resolver_file_content, resolver_file_path,
    uninstall_via_osascript,
};
