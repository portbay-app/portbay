# compiled/

The **committed, build-ready** macOS asset catalog.

`Assets.car` here is compiled from `../PortBay.icon` (Icon Composer) by
`scripts/prepare-macos-liquid-glass-icon.sh` and wired statically into the app
bundle via `tauri.conf.json`:

```jsonc
"bundle": {
  "resources": { "icons/macos-liquid-glass/compiled/Assets.car": "Assets.car" }
}
```

together with `src-tauri/Info.plist` (`CFBundleIconName=PortBay`, which Tauri v2
merges). Because the catalog is committed, **every build — local and CI, on any
runner including macos-14 without Xcode 26 — ships the Liquid Glass icon** with
no build-time step.

## Regenerating

Run on macOS 15+ with Xcode 26 selected, then commit the result:

```
pnpm tauri:icon:macos:build      # = scripts/prepare-macos-liquid-glass-icon.sh
git add src-tauri/icons/macos-liquid-glass/compiled/Assets.car
```

`scripts/inject-macos-liquid-glass-icon.sh` (`pnpm tauri:icon:macos`) is the
separate POST-build tool that re-signs an already-built `.app` for quick local
preview; it also reads this catalog verbatim when present.

```
compiled/
└── Assets.car   ← committed binary; regenerate + re-commit when the icon changes
```
