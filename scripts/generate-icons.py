#!/usr/bin/env python3
"""
Generate a multi-platform icon set for FunctionFly Vault from a brand mark.

Outputs (into `src-tauri/icons/`):
    - icon.png              (1024x1024 PNG, master; Tauri reads this for the .deb bundle)
    - 32x32.png             (Tauri/Linux AppImage banner)
    - 128x128.png           (Tauri/Linux AppImage banner)
    - 128x128@2x.png        (Tauri/Linux AppImage banner; 256x256)
    - Square30x30Logo.png    (Windows MSI small tile)
    - Square44x44Logo.png    (Windows MSI medium tile)
    - Square71x71Logo.png    (Windows MSI large tile)
    - Square89x89Logo.png    (Windows MSI store tile)
    - Square107x107Logo.png  (Windows MSI large tile, alt)
    - Square142x142Logo.png  (Windows MSI large tile)
    - Square150x150Logo.png  (Windows MSI large tile)
    - Square284x284Logo.png  (Windows MSI large tile, 2x)
    - Square310x310Logo.png  (Windows MSI large tile, 2x alt)
    - StoreLogo.png          (Windows MSI store logo, 50x50)

Run from the vault-desktop directory:
    python3 scripts/generate-icons.py
"""

from __future__ import annotations

import os
import sys
from pathlib import Path

try:
    from PIL import Image, ImageDraw, ImageFilter
except ImportError:
    sys.exit(
        "Pillow is required: pip3 install --break-system-packages --user Pillow"
    )

# Brand colours — match the dashboard token in src/index.css
BRAND_BG = (10, 13, 17, 255)            # near-black
BRAND_ACCENT = (94, 106, 210, 255)        # indigo/violet
BRAND_RING = (134, 146, 234, 230)         # accent outline
BRAND_INNER = (255, 255, 255, 235)       # keyway face

ICON_DIR = Path(__file__).resolve().parent.parent / "src-tauri" / "icons"

# Required by tauri.conf.json bundle configuration
WINDOWS_TILES = [
    ("Square30x30Logo.png", 30),
    ("Square44x44Logo.png", 44),
    ("Square71x71Logo.png", 71),
    ("Square89x89Logo.png", 89),
    ("Square107x107Logo.png", 107),
    ("Square142x142Logo.png", 142),
    ("Square150x150Logo.png", 150),
    ("Square284x284Logo.png", 284),
    ("Square310x310Logo.png", 310),
    ("StoreLogo.png", 50),
]

LINUX_PNGS = [
    ("32x32.png", 32),
    ("128x128.png", 128),
    ("128x128@2x.png", 256),
]


def make_master(size: int = 1024) -> Image.Image:
    """Draw a brand mark: rounded square + concentric keyway ring + centre key."""
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)

    # Rounded square background
    radius = int(size * 0.22)
    draw.rounded_rectangle(
        ((0, 0), (size - 1, size - 1)),
        radius=radius,
        fill=BRAND_BG,
    )

    # Outer ring (vault)
    pad = int(size * 0.18)
    ring_w = max(2, int(size * 0.025))
    draw.ellipse(
        ((pad, pad), (size - pad, size - pad)),
        outline=BRAND_RING,
        width=ring_w,
    )

    # Inner accent ring
    pad2 = int(size * 0.27)
    ring2_w = max(2, int(size * 0.015))
    draw.ellipse(
        ((pad2, pad2), (size - pad2, size - pad2)),
        outline=BRAND_ACCENT,
        width=ring2_w,
    )

    # Centre keyway: vertical rounded rect  with a small hole at the bottom
    cx = size // 2
    cy = size // 2
    stem_h = int(size * 0.32)
    stem_w = int(size * 0.13)
    stem_top = cy - stem_h // 2
    stem_bot = cy + stem_h // 2

    # stem
    draw.rounded_rectangle(
        ((cx - stem_w // 2, stem_top), (cx + stem_w // 2, stem_bot)),
        radius=int(stem_w // 2),
        fill=BRAND_INNER,
    )

    # top bow
    bow_r = int(stem_w * 0.95)
    bow_cx = cx
    bow_cy = stem_top
    draw.ellipse(
        ((bow_cx - bow_r, bow_cy - bow_r), (bow_cx + bow_r, bow_cy + bow_r)),
        outline=BRAND_INNER,
        width=max(2, int(size * 0.018)),
    )

    # bottom teeth
    tooth_h = int(size * 0.06)
    tooth_w = int(size * 0.06)
    draw.rectangle(
        ((cx - stem_w // 2 - 1, stem_bot - tooth_h),
         (cx - stem_w // 2 + tooth_w, stem_bot + 1)),
        fill=BRAND_INNER,
    )
    draw.rectangle(
        ((cx + stem_w // 2 - tooth_w, stem_bot - tooth_h),
         (cx + stem_w // 2 + 1, stem_bot + 1)),
        fill=BRAND_INNER,
    )

    # Pinhole in the bow
    pin_r = int(size * 0.045)
    draw.ellipse(
        ((bow_cx - pin_r, bow_cy - pin_r), (bow_cx + pin_r, bow_cy + pin_r)),
        fill=BRAND_BG,
    )

    # Subtle soft glow on the rings so they don't look flat
    glow = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    gd = ImageDraw.Draw(glow)
    gd.ellipse(
        ((pad - 4, pad - 4), (size - pad + 4, size - pad + 4)),
        outline=(94, 106, 210, 70),
        width=2,
    )
    glow = glow.filter(ImageFilter.GaussianBlur(radius=size // 100))
    img = Image.alpha_composite(img, glow)

    return img


def write_icon(img: Image.Image, name: str, size: int) -> Path:
    out = ICON_DIR / name
    img.resize((size, size), Image.LANCZOS).save(out, "PNG", optimize=True)
    return out


def write_ico(icns_path: Path) -> Path:
    """Write the Windows .ico by re-encoding the .icns we already made.

    Pillow's ICO writer refuses `append_images` for multi-frame output;
    but it does honor a multi-resolution container when the input is an
    Apple ICNS file (whose native encoder produces all the canonical
    sizes). Loading the .icns back and saving as ICO yields a valid
    multi-frame icon with the 16/24/32/48/64/128/256 sizes that
    Windows MSI bundlers expect.
    """
    out = ICON_DIR / "icon.ico"
    icns = Image.open(icns_path)
    icns.save(out, format="ICO")
    return out


def write_icns(img: Image.Image) -> Path:
    """Write the macOS .icns. Pillow's ICNS encoder handles the 1024 master."""
    out = ICON_DIR / "icon.icns"
    img.resize((1024, 1024), Image.LANCZOS).save(out, "ICNS")
    return out


def main() -> int:
    ICON_DIR.mkdir(parents=True, exist_ok=True)
    print(f"→ writing icons into {ICON_DIR}")

    master = make_master(1024)
    out_paths: list[Path] = []

    # Linux PNGs
    for name, size in LINUX_PNGS:
        p = write_icon(master, name, size)
        out_paths.append(p)
        print(f"  + {p.name} ({size}×{size})")

    # Windows tiles (used by MSI bundle)
    for name, size in WINDOWS_TILES:
        p = write_icon(master, name, size)
        out_paths.append(p)
        print(f"  + {p.name} ({size}×{size})")

    # The canonical icon.png (Linux bundle master)
    master.save(ICON_DIR / "icon.png", "PNG", optimize=True)
    out_paths.append(ICON_DIR / "icon.png")
    print(f"  + icon.png (1024×1024, master)")

    # macOS
    icns = write_icns(master)
    out_paths.append(icns)
    print(f"  + {icns.name}")

    # Windows .ico (multi-resolution, derived from the .icns)
    ico = write_ico(icns)
    out_paths.append(ico)
    print(f"  + {ico.name}")

    # Restore the original 32×32 placeholder if it was the master (it isn't any more)
    print(f"\nGenerated {len(out_paths)} files.")
    print("Run `cd src-tauri && cargo tauri build` to produce installers.")
    return 0


if __name__ == "__main__":
    sys.exit(main())