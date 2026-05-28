# compiled/

Optional, deterministic drop-zone for a **pre-built** macOS asset catalog.

If you place `Assets.car` here (compiled from `../PortBay.icon` via Xcode 26),
`scripts/inject-macos-liquid-glass-icon.sh` will use it verbatim and skip the
on-the-fly `actool` compile. Use this when your release/CI runner does not
have Xcode 26, or when you want a reproducible, reviewed catalog.

```
compiled/
└── Assets.car   ← commit this (binary) after compiling PortBay.icon
```

This folder is intentionally empty until someone compiles the icon. See the
parent `README.md` → "Pre-compiled Assets.car" for how to generate it.
