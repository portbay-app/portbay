# macOS 26 (Tahoe) Liquid Glass app icon

This folder holds the **source of truth** for PortBay's modern macOS app
icon: an Apple **Icon Composer** document, `PortBay.icon`.

On macOS 26 the system renders the icon's rim, specular shine, edge
highlight, translucency, shadow, and the **Default / Dark / Clear-Light /
Clear-Dark / Tinted** appearances itself, from this single document. None of
that is drawn by PortBay's Rust/Tauri code, and it must not be — faking the
glass treatment in code would drift from whatever Apple ships. We author the
design once here; the OS does the rest.

> The runtime Dock-tile swap in `src-tauri/src/dock_icon.rs` is a separate,
> pre-Tahoe nicety (it swaps the *running* app's Dock image between a light
> and dark PNG). It is **not** the Liquid Glass pipeline and does not touch
> these assets. On macOS 26 the system icon below is the real source of truth.

## What's here

```
macos-liquid-glass/
├── PortBay.icon/              ← Icon Composer document (edit this)
│   ├── icon.json              ← layer/appearance manifest
│   └── Assets/portbay-logo.png ← the foreground glyph (512×512 RGBA)
├── compiled/                  ← optional: pre-built Assets.car (see below)
└── README.md                  ← this file
```

## The five appearances

Icon Composer derives **all five** macOS 26 appearances from this one
document. You do not maintain five separate image files — you author the
design once and, per appearance, optionally override a layer's fill, blur,
or visibility inside Icon Composer.

| Appearance      | How it's produced today                                              |
|-----------------|----------------------------------------------------------------------|
| **Default / Light** | The `fill.solid` background + the glass `portbay-logo` layer in `icon.json`. |
| **Dark**        | Auto-derived by the system from the same layers. To customize, add a Dark appearance override in Icon Composer. |
| **Clear Light** | Auto-derived (system removes the fill, glass-tints the glyph). Override in Icon Composer if needed. |
| **Clear Dark**  | Auto-derived. Override in Icon Composer if needed. |
| **Tinted**      | Auto-derived monochrome treatment. Provide a clean, high-contrast glyph for best results; tweak in Icon Composer's Tinted preview. |

To change any appearance, open `PortBay.icon` in **Icon Composer** (bundled
with Xcode 26), switch the appearance in the toolbar, and adjust. Do not
hand-edit `icon.json` for appearance art — let the tool write it.

## Build / release workflow

Tauri cannot consume a `.icon` document directly. Its bundler only embeds the
static `icon.icns` listed in `src-tauri/tauri.conf.json` → `bundle.icon`. That
`.icns` stays as the **macOS 11–15 fallback**. The macOS 26 catalog reaches the
`.app` by being **committed and wired statically** — so every build, local and
CI, bakes it in with no build-time step.

### How it's wired (committed + static)

The compiled catalog is committed at `compiled/Assets.car` and referenced
directly from `src-tauri/tauri.conf.json`:

```jsonc
"bundle": {
  "resources": { "icons/macos-liquid-glass/compiled/Assets.car": "Assets.car" }
}
```

Tauri copies it to `Contents/Resources/Assets.car` during assembly. Alongside
it, `src-tauri/Info.plist` sets `CFBundleIconName = PortBay`, which Tauri v2
merges into the generated `Info.plist`. Because `tauri build` compiles → signs
→ notarises → packages in one shot, having the catalog present as a committed
build input means it's signed and stapled like any other resource — no re-sign,
no pre-build script, nothing dirtied in the working tree.

```yaml
# .github/workflows/release.yml (macOS job) — no icon step needed
- name: Build signed app
  run: pnpm tauri build --target aarch64-apple-darwin
```

> This works on **any runner, including the default `macos-14`** (which cannot
> run Xcode 26), precisely because the catalog is committed rather than compiled
> in CI. Regenerating the catalog (below) is the only step that needs Xcode 26,
> and it's a local dev action whose output is committed.

### Local preview — inject AFTER building (`inject-…`)

For a quick local look (not for distribution), build then inject + ad-hoc
re-sign:

```bash
pnpm tauri build
pnpm tauri:icon:macos        # scripts/inject-macos-liquid-glass-icon.sh
```

This copies `Assets.car` into the already-built `PortBay.app`, sets
`CFBundleIconName`, and re-signs (ad-hoc unless `APPLE_SIGNING_IDENTITY` is
set). Fine for previewing the icon without a full rebuild; **do not ship** a
bundle injected this way — released builds bake the committed catalog in
directly (see "How it's wired" above).

### Regenerating `compiled/Assets.car`

The catalog is committed, so this is only needed when the icon art changes. Run
on macOS 15+ with Xcode 26 selected, then commit the result:

```bash
pnpm tauri:icon:macos:build      # scripts/prepare-macos-liquid-glass-icon.sh
git add src-tauri/icons/macos-liquid-glass/compiled/Assets.car
```

`prepare-macos-liquid-glass-icon.sh` runs `actool` against `PortBay.icon`
(`--minimum-deployment-target 11.0` so the catalog also carries an `.icns`
fallback) and writes the result straight to `compiled/Assets.car`. It does not
touch `tauri.conf.json` or `Info.plist` — those are committed and static.

## Limitation summary

- Tauri has **no** native `.icon` support and emits no asset catalog — tracked
  upstream (tauri-apps/tauri does not yet bundle Icon Composer documents).
- We work around it by committing a pre-compiled `Assets.car` and registering it
  as a `bundle.resources` entry, so `tauri build` bakes, signs and staples it.
- The fallback `.icns` in `bundle.icon` keeps macOS 11–15 showing a correct, if
  non-glass, icon.
