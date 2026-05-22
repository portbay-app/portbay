//! Mailpit adapter — local SMTP catcher + web UI for outgoing mail
//! from Laravel / Symfony / Rails projects.
//!
//! Same lifecycle shape as `caddy/` and `dnsmasq/`: one bundled
//! sidecar (or PATH fallback), spawned on app boot, killed on window
//! close. Two listening ports: SMTP and a web UI. Both bound to
//! `127.0.0.1` so the catcher never accepts mail from outside the
//! local machine.
//!
//! The web UI is intentionally not opened automatically — the GUI's
//! "Open inbox" action is wired separately (a follow-up card).

pub mod error;
pub mod lifecycle;

pub use error::{MailpitError, Result};
pub use lifecycle::{
    binary_available, find_free_port, MailpitSidecar, DEFAULT_SMTP_PORT, DEFAULT_UI_PORT,
    PORT_SCAN_RANGE,
};
