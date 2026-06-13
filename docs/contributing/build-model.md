# Build Model: Community vs Pro

PortBay Community is the whole app, open source under AGPL-3.0 — **minus** a
small set of proprietary features that live in a private overlay. This page
explains what compiles in the public build, what's gated off, and why you'll
see feature flags whose implementation isn't in the repo. Read it once and the
`#[cfg(feature = "...")]` lines stop being mysterious.

The short version: **clone, build, run — the public build is fully functional.**
The gated features are conveniences (a task board, a visual editor), not the
core. Nothing you need to contribute to the open-source app is missing.

---

## The overlay model

The public repository is the complete Community app. The proprietary Pro
features are **not** stripped out and stubbed by hand — they're guarded behind
Cargo features that are simply **off** in the public build. Their source is
injected at build time by the official release overlay
(`portbay-cloud/desktop-pro`), which compiles with those features on.

In `src-tauri/src/lib.rs` you'll find registration sites guarded like this:

```rust
#[cfg(feature = "tasks")]
mod context;            // Pro: per-project task board + context engine
```

There are ~83 `#[cfg(feature = "tasks")]` and ~49 `#[cfg(feature = "visual-editor")]`
sites across `lib.rs`. In the public build every one of them compiles to
nothing — the modules, Tauri commands, and routes they register don't exist, so
the `/tasks` and visual-editor surfaces are simply absent. The app runs normally
without them.

**This is expected, not a missing piece.** If you can't find the implementation
behind a `tasks` or `visual-editor` gate, you're not looking in the wrong place —
it isn't in this repo, and it shouldn't be submitted to it (see
[license-policy.md](license-policy.md)).

---

## Cargo features

Defined in `src-tauri/Cargo.toml`. None are in `default`; the public build and
CI compile with `--no-default-features`.

| Feature | Off in OSS? | What it gates |
|---|---|---|
| `mcp` | On only for the MCP sidecar | The MCP server protocol stack (`portbay_lib::mcp`, `rmcp`). Enabled when building `crates/mcp` (`scripts/build-mcp.sh`), not for the GUI/CLI. |
| `tasks` | **Yes — Pro** | The per-project task board (`crate::context`, `commands::tasks`, the notification scanner) and the repo-map context engine — which pulls in `tree-sitter` plus eight grammar crates that compile C parsers. The public build links none of them. |
| `visual-editor` | **Yes — Pro** | The embedded live-preview child webview (Tauri `unstable` multiwebview), the browser edit-mode injection proxy, and native WKWebView snapshot/OCR. |
| `pro` | **Yes — Pro** | Umbrella: `["tasks", "visual-editor"]`. `scripts/dev-pro.sh` and `scripts/release-dmg-local.sh` pass `--features pro` — but that only compiles with the private overlay source present. |

As a contributor you build the default (none of the above), and that's the build
CI gates. You do not need the Pro overlay to develop, test, or ship a fix.

---

## Platform: macOS-only, for now

PortBay targets **macOS on Apple Silicon**. CI runs on `macos-14`. Linux and
Windows are roadmap items (see [linux-support-memo.md](../linux-support-memo.md)) —
there's no Linux CI and no supported Linux build yet.

Several sidecars are Swift/CoreML and build with **SwiftPM on macOS 14+**:

| Script | Builds | Notes |
|---|---|---|
| `scripts/build-stt.sh` | Speech-to-text sidecar (WhisperKit + FluidAudio) | SwiftPM release build; first build clones and compiles both engines (slow). |
| `scripts/build-capture.sh` | Screen-capture / selection sidecar | SwiftPM, AppKit. |
| `scripts/build-afm.sh` | Apple Intelligence bridge | Bare `swiftc`. |

Each script checks `uname -s` and **exits cleanly (no-op) on non-Darwin**, so
shared dev/CI flows can call them unconditionally. The Swift binaries are listed
in `tauri.macos.conf.json`'s `externalBin` (not the base `tauri.conf.json`), so
non-macOS bundles never look for them.

What this means in practice:

- **macOS 14+** — everything builds and runs.
- **macOS 13** — the core app builds, but the Swift sidecars won't (the SwiftPM
  floor is macOS 14). You lose speech-to-text, text-to-speech, screen capture,
  and the on-device Apple Intelligence paths; projects, HTTPS, databases, DNS,
  tunnels, SSH, and MCP still work.
- **Linux** — the fetch/build scripts no-op the macOS-only pieces, but there is
  no supported build target yet; `pnpm tauri dev` is not expected to come up.

The macOS-only native code is gated with `#[cfg(target_os = "macos")]`, and
some subsystems (hosts helper, notifications, vibrancy) already carry Linux
branches for when the platform lands.

---

## Where to go next

- [Development setup](development.md) — prerequisites, clone, fetch sidecars, run, CI gates.
- [Architecture orientation](architecture.md) — the Tauri + Rust core and the cloud-client boundary.
- [License policy](license-policy.md) — what must never be submitted, and the repo-boundary denylist.
