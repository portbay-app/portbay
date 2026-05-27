//! On-demand managed-runtime delivery (the Herd model).
//!
//! PortBay ships a small DMG and never runs a competitor's binary, so for PHP
//! (and, later, nginx/apache + database engines) it must provide **its own**
//! runtimes. Bundling every build in the DMG would balloon it to hundreds of MB
//! and reship all of it on each update; instead PortBay fetches its own lean,
//! self-built, signed runtimes on demand into
//! `Application Support/PortBay/runtimes/<lang>/<version>/` and runs them as
//! managed sidecars.
//!
//! This module is the trust + transport layer for that delivery:
//! - [`manifest`] — the signed catalogue of downloadable runtimes plus its
//!   verify-and-parse gate (pure, no network). Every download must pass it.
//! - [`install`] — the download/extract manager: fetch → verify size + SHA-256
//!   → decompress (zstd) + unpack (tar) into a staging dir → probe the expected
//!   binary → atomically rename into place. The verify/extract/install core is
//!   pure (operates on in-memory bytes); only the thin fetch wrapper touches the
//!   network.
//!
//! The managed-runtime registry that makes `resolve_binary` prefer these builds
//! ships in `super` (`InstallSource::PortBay`, `RuntimeSettings.managed`).

pub mod install;
pub mod manifest;
