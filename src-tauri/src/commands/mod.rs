//! Tauri command surface — the IPC boundary between the Rust core and the
//! Svelte frontend.
//!
//! One file per concern; one `tauri::generate_handler!` registration in
//! `lib.rs` aggregates them. The generate_handler! macro must be called
//! exactly once, so there's no "modules contribute their own handlers"
//! pattern — the flat list at the call site is the price of compile-time
//! wiring.
//!
//! Frontend contract: every command returns `Result<T, AppError>`, where
//! `AppError` serialises into the §5.4 envelope shape. See
//! `src/error.rs` for the exact wire format.

pub mod artifacts;
pub mod auth;
pub mod certs;
pub mod databases;
pub mod dbconn;
pub mod dnsmasq;
pub mod dto;
pub mod entitlements;
pub mod events;
pub mod groups;
pub mod http_inspector;
pub mod import;
pub mod integrations;
pub mod lifecycle;
pub mod log_stream;
pub mod metrics;
pub mod onboarding;
pub mod portfile;
pub mod preferences;
pub mod projects;
pub mod runtimes;
pub mod sidecars;
pub mod sync;
pub mod system;
pub mod telemetry;
pub mod tunnel;
pub mod updater;
pub mod webservers;
