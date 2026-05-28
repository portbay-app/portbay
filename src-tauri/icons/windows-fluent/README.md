# Windows — Fluent / Mica icon track (placeholder)

**Status: placeholder. Windows currently ships `../icon.ico` from the default
Tauri `bundle.icon` array — that is fully working and is not affected by this
folder.**

This folder reserves space for a future Windows 11 **Fluent / Mica**-aware
icon set, should we decide to tailor the Windows icon beyond a single `.ico`.
Likely contents when that work happens:

- Unplated source art (transparent, no background plate) so Windows can apply
  its own tile/acrylic background.
- Light and dark Start-menu / taskbar tile variants.
- High-DPI PNG sizes (16–256 px) feeding a regenerated multi-resolution
  `../icon.ico`.

Nothing here is consumed by the build yet. Drop assets in and wire them into
`scripts/generate-icons.py` (which produces `../icon.ico`) when prioritized.
