//! Cloudflare Tunnel — "share this localhost URL publicly" via the
//! bundled `cloudflared` sidecar.
//!
//! Per-project ephemeral tunnels (no Cloudflare account required).
//! Spawning `cloudflared tunnel --url <upstream> --no-autoupdate
//! --no-tls-verify` connects to Cloudflare's edge and assigns a
//! `https://<random>.trycloudflare.com` URL announced on stdout. We
//! capture stdout, parse the URL, and surface it to the GUI.
//!
//! `TunnelManager` keeps one `Tunnel` per project. Stopping the tunnel
//! kills the child; `TunnelManager` is `Drop`-clean so app shutdown
//! sweeps any active tunnels.

pub mod error;
pub mod lifecycle;

pub use error::{Result, TunnelError};
pub use lifecycle::{wait_for_url, Tunnel, TunnelManager, TunnelStatus, TUNNEL_URL_TIMEOUT};
