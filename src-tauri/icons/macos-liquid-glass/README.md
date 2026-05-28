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
`.app` via one of two scripts, depending on whether the bundle gets signed.

### Release / signed builds — bake in BEFORE signing (`prepare-…`)

`tauri build` compiles → signs → notarises → packages the `.dmg` and updater
`.app.tar.gz`(+`.sig`) in one shot. The catalog must therefore be present
*before* Tauri signs, or the notarisation staple and updater signature would
be invalidated and the packaged artifacts would still hold the un-injected
`.app`. So the release workflow runs the **pre-build** bake immediately before
`tauri build`:

```yaml
# .github/workflows/release.yml (macOS job)
- name: Bake macOS 26 Liquid Glass icon (pre-build)
  run: bash scripts/prepare-macos-liquid-glass-icon.sh
- name: Build signed app
  run: pnpm tauri build --target aarch64-apple-darwin
```

`prepare-macos-liquid-glass-icon.sh` resolves a compiled `Assets.car`
(committed `compiled/Assets.car`, else `actool` on Xcode 26), then **bakes it
into the build inputs** — no re-sign needed:
- registers it as a Tauri **bundle resource** (`bundle.resources`) so Tauri
  copies it to `Contents/Resources/Assets.car` during assembly, and
- writes `CFBundleIconName = PortBay` to `src-tauri/Info.plist`, which Tauri
  v2 merges into the generated `Info.plist`.

These edits land only in the CI workspace (fresh checkout; the staging dir is
git-ignored) — the committed config is never mutated. **Non-fatal:** if no
catalog is available and the runner lacks Xcode 26, it warns and the build
ships the `.icns` fallback rather than failing.

> The default `macos-14` runner cannot run Xcode 26 (needs macOS 15+), so on
> that runner the glass icon ships **only if `compiled/Assets.car` is
> committed**. To compile on the runner instead, move the macOS job to a
> macOS 15+ image with Xcode 26 selected.

### Local preview — inject AFTER building (`inject-…`)

For a quick local look (not for distribution), build then inject + ad-hoc
re-sign:

```bash
pnpm tauri build
pnpm tauri:icon:macos        # scripts/inject-macos-liquid-glass-icon.sh
```

This copies `Assets.car` into the already-built `PortBay.app`, sets
`CFBundleIconName`, and re-signs (ad-hoc unless `APPLE_SIGNING_IDENTITY` is
set). Fine for previewing the icon; **do not ship** a bundle injected this way
— use the pre-build path for releases.

### Producing `compiled/Assets.car`

`actool`'s standalone-`.icon` support is new in Xcode 26 and its exact CLI
flags are still settling — **validate the `actool` call on your build
machine.** For a deterministic, Xcode-free release (works on any runner),
compile once and commit the result:

1. Add `PortBay.icon` to a throwaway Xcode 26 project and build once.
2. Copy the produced `Assets.car` to `compiled/Assets.car` here and commit it.

When `compiled/Assets.car` exists, both scripts use it verbatim and skip
`actool` entirely.

### Clean static end-state (optional)

Once `compiled/Assets.car` is committed and validated, you can drop the
scripts entirely and wire the catalog statically: add a permanent
`src-tauri/Info.plist` with `CFBundleIconName`, and a `bundle.resources` entry
mapping `icons/macos-liquid-glass/compiled/Assets.car` → `Assets.car` in
`tauri.conf.json`. We avoid doing this *now* because a `bundle.resources` entry
pointing at a missing file would break every local `tauri build` until the
catalog is committed.

## Limitation summary

- Tauri has **no** `.icon` support and emits no asset catalog — tracked
  upstream (tauri-apps/tauri does not yet bundle Icon Composer documents).
- Therefore the macOS 26 icon is a **post-build injection**, not part of the
  normal `tauri build` output.
- The fallback `.icns` in `bundle.icon` keeps macOS 11–15 (and any build that
  skips the injection step) showing a correct, if non-glass, icon.
