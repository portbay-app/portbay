#!/usr/bin/env python3
"""Generate the branded DMG installer background for PortBay.

Tauri/create-dmg map the background image's pixels 1:1 to *points* and ignore
DPI metadata (tauri-apps/tauri#12009), so a plain 2x PNG gets cut off and
oversized in the window. The retina-correct fix (create-dmg#176) is a
multi-resolution HiDPI TIFF: a 1x rep sized to the window's point dimensions
plus a 2x rep, combined with `tiffutil -cathidpicheck`. Finder then picks the
2x rep on Retina and the 1x rep elsewhere — crisp everywhere, never cut.

Output: src-tauri/icons/dmg-background.tiff (referenced from tauri.conf.json).
Regenerate any time the palette / wordmark changes:

    python3 scripts/generate-dmg-background.py

The icon-slot coordinates here MUST stay in sync with the `dmg` block in
src-tauri/tauri.conf.json (windowSize / appPosition / applicationFolderPosition).
The two drop slots are left empty — Finder overlays the real PortBay.app icon
and the /Applications symlink there at install time.
"""
import os
import subprocess
import tempfile

from PIL import Image, ImageDraw, ImageFilter, ImageFont

# ── Geometry, in POINTS — the DMG window is 540x380 pt ───────────────────────
W_PT, H_PT = 540, 380

# Keep in sync with tauri.conf.json bundle.macOS.dmg (icon-slot centers).
APP_POS = (130, 220)
APPLICATIONS_POS = (410, 220)

# ── Palette (from src/app.css @theme) ────────────────────────────────────────
BG = (11, 15, 20)          # --color-bg base   #0B0F14
SURFACE = (24, 30, 38)     # slightly lifted    ~#181E26
FG = (230, 237, 243)       # --color-fg         #E6EDF3
FG_MUTED = (139, 148, 158) # --color-fg-muted   #8B949E
ACCENT = (77, 156, 255)    # --color-accent     #4D9CFF

ROOT = os.path.normpath(os.path.join(os.path.dirname(__file__), ".."))
OUT_TIFF = os.path.join(ROOT, "src-tauri/icons/dmg-background.tiff")
# Real macOS Applications folder icon, extracted once via NSWorkspace (see
# scripts/assets/). Baked into the background because macOS 26 (Tahoe) renders
# the /Applications symlink icon blank in Tauri DMGs (tauri-apps/tauri#14500).
APPS_ICON = os.path.join(ROOT, "scripts/assets/applications-folder.png")

# Finder renders DMG icons at 128 pt (bundle_dmg.sh ICON_SIZE=128, not
# overridden by Tauri). Match it so the baked folder lines up with the app icon.
ICON_PT = 128

# Light tile colours. Finder forces *dark* filename labels when a custom
# background is set (create-dmg#197 — white labels aren't settable), so each
# icon sits on a soft light tile that makes its label readable on the dark bg.
TILE_TOP = (244, 247, 250)
TILE_BOTTOM = (223, 229, 236)

FONT_CANDIDATES = [
    "/System/Library/Fonts/Helvetica.ttc",
    "/System/Library/Fonts/HelveticaNeue.ttc",
    "/Library/Fonts/Arial.ttf",
]


def load_font(size: int) -> ImageFont.FreeTypeFont:
    for path in FONT_CANDIDATES:
        if os.path.exists(path):
            try:
                return ImageFont.truetype(path, size)
            except OSError:
                continue
    return ImageFont.load_default()


def lerp(a, b, t):
    return tuple(round(a[i] + (b[i] - a[i]) * t) for i in range(3))


def draw_centered(d, cx, y, text, font, fill, tracking=0):
    if tracking == 0:
        w = d.textlength(text, font=font)
        d.text((cx - w / 2, y), text, font=font, fill=fill)
        return
    widths = [d.textlength(ch, font=font) for ch in text]
    total = sum(widths) + tracking * (len(text) - 1)
    x = cx - total / 2
    for ch, w in zip(text, widths):
        d.text((x, y), ch, font=font, fill=fill)
        x += w + tracking


def draw_icon_tile(img: Image.Image, cx_pt: int, scale: int):
    """Soft light tile behind an icon slot so its (forced-dark) Finder label
    reads on the dark background. Spans the icon plus the label below it."""
    cx = cx_pt * scale
    half_w = 78 * scale
    top, bottom = 146 * scale, 320 * scale
    radius = 22 * scale
    box = [cx - half_w, top, cx + half_w, bottom]

    # Drop shadow — a blurred dark rounded rect, offset down a touch.
    shadow = Image.new("RGBA", img.size, (0, 0, 0, 0))
    sd = ImageDraw.Draw(shadow)
    sd.rounded_rectangle(
        [box[0], box[1] + 6 * scale, box[2], box[3] + 6 * scale],
        radius=radius, fill=(0, 0, 0, 90),
    )
    shadow = shadow.filter(ImageFilter.GaussianBlur(10 * scale))
    img.paste(shadow, (0, 0), shadow)

    # Tile face — vertical light gradient, clipped to the rounded rect.
    h = bottom - top
    grad = Image.new("RGB", (1, h))
    gp = grad.load()
    for i in range(h):
        gp[0, i] = lerp(TILE_TOP, TILE_BOTTOM, i / h)
    grad = grad.resize((box[2] - box[0], h))
    mask = Image.new("L", (box[2] - box[0], h), 0)
    ImageDraw.Draw(mask).rounded_rectangle(
        [0, 0, box[2] - box[0] - 1, h - 1], radius=radius, fill=255)
    img.paste(grad, (box[0], top), mask)

    # Crisp light hairline border.
    ImageDraw.Draw(img).rounded_rectangle(
        box, radius=radius, outline=(255, 255, 255), width=max(1, scale))


def render(scale: int) -> Image.Image:
    """Render the background at `scale`x (1 → 540x380, 2 → 1080x760)."""
    W, H = W_PT * scale, H_PT * scale
    img = Image.new("RGB", (W, H), BG)

    # Vertical depth gradient, BG → a touch toward SURFACE.
    grad = Image.new("RGB", (1, H))
    gpx = grad.load()
    for y in range(H):
        gpx[0, y] = lerp(BG, SURFACE, (y / H) * 0.55)
    img.paste(grad.resize((W, H)))

    # Soft accent glow centred on the icon row, between the two slots.
    glow = Image.new("L", (W, H), 0)
    gd = ImageDraw.Draw(glow)
    cx = ((APP_POS[0] + APPLICATIONS_POS[0]) / 2) * scale
    cy = APP_POS[1] * scale
    max_r = 210 * scale
    for r in range(max_r, 0, -2 * scale):
        a = int(30 * (1 - r / max_r))
        gd.ellipse([cx - r, cy - r, cx + r, cy + r], fill=a)
    img = Image.composite(Image.new("RGB", (W, H), ACCENT), img, glow)

    # Light tiles under both slots (Finder draws the app icon on the left tile;
    # we bake the Applications folder on the right because Tahoe leaves it blank).
    draw_icon_tile(img, APP_POS[0], scale)
    draw_icon_tile(img, APPLICATIONS_POS[0], scale)

    # Bake the real Applications folder icon onto its tile, matching Finder's
    # 128 pt icon size, centred on the slot.
    if os.path.exists(APPS_ICON):
        side = ICON_PT * scale
        apps = Image.open(APPS_ICON).convert("RGBA").resize(
            (side, side), Image.LANCZOS)
        ax = APPLICATIONS_POS[0] * scale - side // 2
        ay = APPLICATIONS_POS[1] * scale - side // 2
        img.paste(apps, (ax, ay), apps)

    d = ImageDraw.Draw(img)

    # Wordmark + tagline, comfortably inside the 540pt width.
    draw_centered(d, W // 2, 36 * scale, "PortBay",
                  load_font(30 * scale), FG, tracking=scale)
    draw_centered(d, W // 2, 82 * scale,
                  "Drag PortBay onto the Applications folder to install",
                  load_font(12 * scale), FG_MUTED)

    # Arrow on the icon row, in the gap between the two tiles.
    y = APP_POS[1] * scale
    x0 = (APP_POS[0] + 86) * scale
    x1 = (APPLICATIONS_POS[0] - 86) * scale
    d.line([(x0, y), (x1 - 9 * scale, y)], fill=ACCENT, width=5 * scale)
    head = 12 * scale
    d.polygon(
        [(x1, y), (x1 - head, y - head), (x1 - head, y + head)],
        fill=ACCENT,
    )
    return img


def main():
    with tempfile.TemporaryDirectory() as tmp:
        p1 = os.path.join(tmp, "bg.png")
        p2 = os.path.join(tmp, "bg@2x.png")
        render(1).save(p1, "PNG", optimize=True)
        render(2).save(p2, "PNG", optimize=True)
        # -cathidpicheck verifies p2 is exactly 2x p1, then writes a HiDPI TIFF.
        subprocess.run(
            ["tiffutil", "-cathidpicheck", p1, p2, "-out", OUT_TIFF],
            check=True,
            stdout=subprocess.DEVNULL,
        )
    print(f"wrote {OUT_TIFF} (HiDPI multi-rep: {W_PT}x{H_PT} pt @ 1x + 2x)")


if __name__ == "__main__":
    main()
