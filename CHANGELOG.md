# Changelog

All notable changes to PortBay are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.4] — 2026-06-09

- On-device AI, expanded: image generation (Stable Diffusion / SDXL via
  Apple's ml-stable-diffusion, plus an Apple Image Playground bridge),
  Kokoro text-to-speech with 28 English voices, Ollama embeddings, and
  playground tabs for every local model type on the AI page.
- Speech-to-Text: local on-device transcription with Whisper and Parakeet
  (v2/v3) on the Neural Engine — a resident engine stays warm between
  captures, live partial transcripts, and model downloads from a signed
  PortBay Model Catalog with provenance (live / cached / built-in).
- Dictate anywhere: hold (or double-tap) Fn in any app and the local engine
  types the transcript where you are — notch overlay HUD, per-app formatting
  contexts, voice commands, entity formatting, custom terms, and
  self-learning vocabulary with a privacy reset.
- Ollama manager: PortBay owns the local server lifecycle (start/stop/
  adopt), with a live library catalog, model store, and board-dispatch
  parity for local-model agents.
- PortBay agent engine: bundled Cline-derived local agent (portbay-agent)
  with worktree-per-card dispatch from the task board.
- Workbench: certificates surface (TLS source fields, create-certificate
  pane), SFTP file-browser overhaul, live streaming deploys with
  cancellation, IDE file-tree with 40+ language modes, sidebar reorder.
- `portbay` CLI installer in Advanced settings (PATH symlink with an
  elevation fallback).
- Fixes: database data dirs healed on start, quieter DNS page when healthy,
  project-selector avatars, and the release workflow never overwrites a
  manually published release.

## [0.1.3] — 2026-06-05

- Smart Dictation: a mic button on task cards and the SSH agent composer
  triggers macOS system dictation (no in-app recognizer), with an optional,
  off-by-default rewrite pass that cleans up the transcript on-device via
  Apple Intelligence (macOS 26+) or a local Ollama model. Push-to-talk via
  the Fn (🌐) key.
- SSH Access redesign: single-pane host workbench (list → detail → form),
  interactive terminal with split panes and broadcast input, smart port
  detection with one-click forwarding, host-key accept dialog, keyboard-
  interactive 2FA, SFTP file manager, remote port-forwarding page.
- ML remote-host tooling: GPU monitoring panel, quick-forward presets for ML
  dashboards, training-run notifications, tmux/screen + SLURM awareness,
  large-file transfer progress with resume, remote CUDA/env awareness.
- Per-project task board with LLM agent dispatch (Claude Code / Codex),
  hand-off documents, agent learnings memory, and MCP task tooling.
- Embedded database client (table browser, query workbench) absorbed into
  Databases and project detail.
- Project-backed certificate settings page; built-in language intelligence
  for the IDE surface.
- Reliability: poison-recovery on all production mutex locks, reference-counted
  sidecar status polling, cache-first PATH bootstrap (no login-shell stall on
  launch), SSH prompt failures surface as toasts, telemetry path scrubbing
  derived at runtime, sync status read without touching the keychain.

## [0.1.2] — 2026-06-04

First release fully built, signed, notarized, and stapled by CI. Incorporates
the unpublished 0.1.1 build. Apple Silicon (`aarch64`) only.

### Added

- Zero-config public sharing of dev servers through Cloudflare quick tunnels,
  with live reachability status (not just process-alive).
- CLI and MCP parity with the app: databases, DNS, runtimes, project groups,
  tunnel teardown, sidecar status probe, and read-only access to certificates,
  logs, and the sandbox.
- PortBay-managed PHP-FPM runtime; nginx and Apache web-server management with
  validated config generation.
- In-app AI integrations panel: MCP setup as an environment picker with
  first-run nudge.
- First-run smoke project serves a local landing page; static sites get real
  play/pause lifecycle.
- Project avatars show the detected favicon or app icon.
- Duplicate hostname and port validation on project add/update.
- macOS polish: appearance-aware Dock icon with a Show-icon-in-Dock toggle,
  redesigned menu-bar tray popover, window vibrancy, new app icon with the
  macOS 26 Liquid Glass pipeline.
- Privacy Policy and Terms of Service pages in the docs site.
- Previously inert settings now work: launch at login, hosts-file management,
  certificate auto-renew, and related toggles.

### Changed

- New projects default to HTTPS, and the dashboard shows the ports Caddy
  actually bound.
- The default domain suffix override is part of Pro.
- Session keychain item renamed to "PortBay Account".
- Docs: all 58 MCP tools documented; DNS and SSL guides synced to behavior.
- Dependency upgrades (thiserror 2, x509-parser 0.18, console 0.16, frontend
  dev dependencies).

### Fixed

- HTTPS lands on port 443 instead of 8443: the wildcard listener is bind-tested
  at boot, TLS terminates via an explicit connection policy, and reconciliation
  pins to the boot-chosen port.
- Public shares no longer fail on origin SNI: tunnels route to the project's
  Caddy listener by scheme and serve over the loopback origin.
- MariaDB is no longer detected as MySQL (shared `mysqld` binary).
- Workspace projects start and stop reliably (`process_compose_id` parity).
- Redis data paths containing spaces work (quoted `dir` and `unixsocket`).
- Bundled Mailpit is detected, dnsmasq is probed over UDP, and port conflicts
  on :80 are attributed to the real holder.
- process-compose receives its environment as a `KEY=value` sequence, fixing a
  daemon crash on boot.
- PortBay never runs competitor binaries (ServBay/Herd/MAMP/XAMPP/FlyEnv) —
  discovery stays read-only; importing their projects remains supported.

### Release infrastructure

- CI builds the DMG signed, notarized, and stapled (`.app` and DMG both), with
  a CycloneDX SBOM and a <100 MB installer budget guard.
- Updater signing key rotated to a password-protected key (2026-05-26).

## [0.1.1] — unreleased

Built and signed by CI on 2026-05-27 but never published; all changes shipped
in 0.1.2.

## [0.1.0] — 2026-05-26

Initial public release.

- Project management for local dev servers: process supervision
  (process-compose), wildcard `.test` DNS (dnsmasq), local HTTPS (mkcert +
  Caddy), hosts-file helper, per-project domains, logs, and metrics.
- Managed runtimes and databases (MySQL, PostgreSQL, Redis), Mailpit mail
  catcher, migration import from Herd, ServBay, and MAMP.
- Pro accounts: GitHub/email sign-in, signed entitlements with offline grace,
  project-cap tiers.
- CLI (`portbay`) and MCP server for agent-driven workflows.
- Signed and notarized macOS build for Apple Silicon.

[0.1.4]: https://github.com/portbay-app/portbay/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/portbay-app/portbay/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/portbay-app/portbay/compare/v0.1.0...v0.1.2
[0.1.1]: https://github.com/portbay-app/portbay/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/portbay-app/portbay/releases/tag/v0.1.0
