# PortBay app icons

`tauri.conf.json` (`bundle.icon`) is strict JSON and cannot carry comments,
so the per-platform notes live here. There are **three tracks**:

## Cross-platform default (Tauri-generated, the working baseline)

The `bundle.icon` array drives Tauri's built-in icon embedding. Do not break
this — it is what makes Windows and Linux icons work, and it provides the
macOS pre-Tahoe fallback:

| File                              | Consumed by                                  |
|-----------------------------------|----------------------------------------------|
| `32x32.png`, `128x128.png`, `128x128@2x.png` | **Linux** (freedesktop PNG sizes) + Tauri window icon |
| `icon.icns`                       | **macOS** fallback (macOS 11–15, and any build that skips the macOS-26 injection step) |
| `icon.ico`                        | **Windows** application icon                 |

Regenerate these with `scripts/generate-icons.py`. They are the source for
every platform that does not have a dedicated modern track below.

## macOS — Liquid Glass / Icon Composer (source of truth on macOS 26)

→ See [`macos-liquid-glass/`](macos-liquid-glass/README.md).

The macOS 26 (Tahoe) icon — rim, shine, translucency, and the
Default/Dark/Clear/Tinted appearances — is authored in Apple **Icon Composer**
(`macos-liquid-glass/PortBay.icon`) and injected into the built `.app` by
`scripts/inject-macos-liquid-glass-icon.sh`. The glass treatment is rendered
by macOS, never synthesized in Rust. The `icon.icns` above remains the
fallback for older macOS.

## Windows — future Fluent / Mica icon track (placeholder)

→ See [`windows-fluent/`](windows-fluent/README.md).

Today Windows uses `icon.ico` from the default array above. The placeholder
folder is where a future Windows 11 Fluent/Mica-aware icon set (unplated
SVG/PNG layers, light/dark tile assets) would live if/when we tailor the
Windows icon beyond a single `.ico`.

## Linux — standard PNG / freedesktop track (placeholder)

→ See [`linux-freedesktop/`](linux-freedesktop/README.md).

Today Linux uses the PNG sizes from the default array, which Tauri installs
into the freedesktop hicolor theme. The placeholder folder documents where
additional sizes or a scalable SVG would go if we extend Linux theming.
