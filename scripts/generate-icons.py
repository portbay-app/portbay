#!/usr/bin/env python3
"""Generate all PortBay app icons from the source tugboat PNG.

macOS/iOS 26 compliance: solid background, 1024×1024 source, no transparency.
"""
import os
import shutil
import subprocess
import tempfile
from PIL import Image, ImageDraw

SRC = "/Users/nour/.claude/image-cache/34f43a76-6734-427b-bc1a-791b3ed8a863/1.png"
ICONS_DIR = "/Volumes/DevSSD/projects/Clients/portbay/src-tauri/icons"
ASSETS_DIR = "/Volumes/DevSSD/projects/Clients/portbay/src/lib/assets"

# iOS 26 / macOS compliant background — clean white matches the tugboat's own white body
BG_COLOR = (255, 255, 255, 255)
# Padding as fraction of canvas size — 5% so the tugboat has a little breathing room
PADDING_FRACTION = 0.05


def make_with_bg(src_img: Image.Image, size: int) -> Image.Image:
    """Composite src onto a solid background, scaled with padding."""
    canvas = Image.new("RGBA", (size, size), BG_COLOR)
    pad = int(size * PADDING_FRACTION)
    inner = size - 2 * pad
    logo = src_img.resize((inner, inner), Image.LANCZOS)
    canvas.paste(logo, (pad, pad), logo)
    return canvas.convert("RGB")  # strip alpha — required for ico/icns


def make_transparent(src_img: Image.Image, size: int) -> Image.Image:
    """Resize preserving transparency — used for in-app assets."""
    return src_img.resize((size, size), Image.LANCZOS)


def save_png(img: Image.Image, path: str):
    img.save(path, "PNG", optimize=True)
    print(f"  wrote {path} ({img.size[0]}x{img.size[1]})")


def main():
    os.makedirs(ASSETS_DIR, exist_ok=True)

    src = Image.open(SRC).convert("RGBA")
    print(f"Source: {src.size[0]}x{src.size[1]} RGBA")

    # ── In-app sidebar logo (transparent, no background) ─────────────────────
    print("\n── In-app assets ──")
    sidebar = make_transparent(src, 128)
    save_png(sidebar, f"{ASSETS_DIR}/portbay-logo.png")

    # ── Tauri / OS icons (with background) ───────────────────────────────────
    print("\n── Tauri icons ──")

    sizes = {
        "32x32.png": 32,
        "128x128.png": 128,
        "128x128@2x.png": 256,
        "icon.png": 512,
        # Windows Store
        "Square30x30Logo.png": 30,
        "Square44x44Logo.png": 44,
        "Square71x71Logo.png": 71,
        "Square89x89Logo.png": 89,
        "Square107x107Logo.png": 107,
        "Square142x142Logo.png": 142,
        "Square150x150Logo.png": 150,
        "Square284x284Logo.png": 284,
        "Square310x310Logo.png": 310,
        "StoreLogo.png": 50,
    }

    for filename, size in sizes.items():
        img = make_with_bg(src, size)
        save_png(img, f"{ICONS_DIR}/{filename}")

    # ── macOS .icns ───────────────────────────────────────────────────────────
    print("\n── macOS .icns ──")
    iconset_dir = tempfile.mkdtemp(suffix=".iconset")
    try:
        icns_sizes = [16, 32, 64, 128, 256, 512, 1024]
        for px in icns_sizes:
            img = make_with_bg(src, px)
            half = px // 2
            # 1× name uses half the pixel count, 2× uses the full count
            if half >= 16:
                save_png(img, f"{iconset_dir}/icon_{half}x{half}@2x.png")
            save_png(img, f"{iconset_dir}/icon_{px}x{px}.png")

        out_icns = f"{ICONS_DIR}/icon.icns"
        subprocess.run(
            ["iconutil", "-c", "icns", iconset_dir, "-o", out_icns],
            check=True,
        )
        print(f"  wrote {out_icns}")
    finally:
        shutil.rmtree(iconset_dir)

    # ── Windows .ico ──────────────────────────────────────────────────────────
    print("\n── Windows .ico ──")
    ico_sizes = [16, 32, 48, 64, 128, 256]
    ico_frames = [make_with_bg(src, s) for s in ico_sizes]
    ico_path = f"{ICONS_DIR}/icon.ico"
    # PIL .ico requires RGB or RGBA; save as RGBA via convert
    ico_frames[0].convert("RGBA").save(
        ico_path,
        format="ICO",
        sizes=[(s, s) for s in ico_sizes],
        append_images=[f.convert("RGBA") for f in ico_frames[1:]],
    )
    print(f"  wrote {ico_path}")

    print("\nDone.")


if __name__ == "__main__":
    main()
