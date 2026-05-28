# Linux — standard PNG / freedesktop icon track (placeholder)

**Status: placeholder. Linux currently ships the `../32x32.png`,
`../128x128.png`, and `../128x128@2x.png` files from the default Tauri
`bundle.icon` array — that is fully working and is not affected by this
folder.** Tauri installs those into the freedesktop **hicolor** icon theme.

This folder reserves space for extending Linux theming if needed:

- Additional hicolor sizes (16, 22, 24, 48, 64, 256 px) for crisper rendering
  across desktop environments and panels.
- A scalable `portbay.svg` for `hicolor/scalable/apps/`.
- A symbolic monochrome variant for GNOME/KDE panel use.

Nothing here is consumed by the build yet. Add assets and extend
`scripts/generate-icons.py` plus the `bundle.icon` array when prioritized.
