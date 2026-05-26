#!/usr/bin/env python3
"""Generate the social/Open Graph share image for the web demo (try.portbay.app).

Output: static/og-image.png (1200x630, the OG/Twitter standard). Referenced from
the simulator <svelte:head> in src/routes/+layout.svelte. Regenerate with:

    python3 scripts/generate-og-image.py
"""
import os

from PIL import Image, ImageDraw, ImageFont

W, H = 1200, 630

# Palette (brand dark + ZFlow blue primary ~ oklch(0.6225 0.2041 259.9)).
BG = (11, 15, 20)
SURFACE = (22, 27, 34)
FG = (230, 237, 243)
FG_MUTED = (139, 148, 158)
ACCENT = (59, 130, 246)

ROOT = os.path.normpath(os.path.join(os.path.dirname(__file__), ".."))
LOGO = os.path.join(ROOT, "src/lib/assets/portbay-logo.png")
OUT = os.path.join(ROOT, "static/og-image.png")

FONT_CANDIDATES = [
    "/System/Library/Fonts/Helvetica.ttc",
    "/System/Library/Fonts/HelveticaNeue.ttc",
    "/Library/Fonts/Arial.ttf",
]


def font(size: int, index: int = 0) -> ImageFont.FreeTypeFont:
    for path in FONT_CANDIDATES:
        if os.path.exists(path):
            try:
                return ImageFont.truetype(path, size, index=index)
            except OSError:
                continue
    return ImageFont.load_default()


def lerp(a, b, t):
    return tuple(round(a[i] + (b[i] - a[i]) * t) for i in range(3))


def main():
    img = Image.new("RGB", (W, H), BG)

    # Diagonal depth gradient BG -> SURFACE.
    grad = Image.new("RGB", (1, H))
    gp = grad.load()
    for y in range(H):
        gp[0, y] = lerp(BG, SURFACE, (y / H) * 0.7)
    img.paste(grad.resize((W, H)))

    # Accent glow behind the logo (left third).
    glow = Image.new("L", (W, H), 0)
    gd = ImageDraw.Draw(glow)
    cx, cy, max_r = 320, H // 2, 360
    for r in range(max_r, 0, -4):
        gd.ellipse([cx - r, cy - r, cx + r, cy + r],
                   fill=int(40 * (1 - r / max_r)))
    img = Image.composite(Image.new("RGB", (W, H), ACCENT), img, glow)

    # Tugboat logo, left.
    if os.path.exists(LOGO):
        side = 360
        logo = Image.open(LOGO).convert("RGBA").resize((side, side), Image.LANCZOS)
        img.paste(logo, (140, (H - side) // 2), logo)

    d = ImageDraw.Draw(img)
    tx = 560
    d.text((tx, 196), "PortBay", font=font(96), fill=FG)
    d.text((tx, 312), "Local development environment", font=font(34), fill=FG_MUTED)
    d.text((tx, 356), "manager for macOS", font=font(34), fill=FG_MUTED)

    # Accent rule + URL.
    d.rectangle([tx, 424, tx + 56, 428], fill=ACCENT)
    d.text((tx, 446), "Automatic HTTPS · .test domains · one-click start",
           font=font(22), fill=FG_MUTED)
    d.text((tx, 482), "try.portbay.app", font=font(26), fill=ACCENT)

    img.save(OUT, "PNG", optimize=True)
    print(f"wrote {OUT} ({W}x{H})")


if __name__ == "__main__":
    main()
