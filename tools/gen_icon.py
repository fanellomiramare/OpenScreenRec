#!/usr/bin/env python3
"""Generate the OpenScreenRec icons.

Design: a flat pastel record dot on a cream squircle (app) and a bare
dot for the tray — sober, flat, minimal by intention. Everything is
drawn with Pillow and supersampled, so the whole identity is
reproducible from this one script:

    python tools/gen_icon.py

and commit the outputs under assets/.
"""

from __future__ import annotations

import struct
from io import BytesIO
from pathlib import Path

from PIL import Image, ImageDraw

CREAM = (247, 232, 224, 255)  # #F7E8E0
CORAL = (226, 92, 92, 255)  # #E25C5C

SIZES = [16, 24, 32, 48, 64, 128, 256]
ICNS_SIZES = [16, 32, 128, 256]
SS = 4  # supersampling factor: draw 4x, downscale for clean antialiasing

REPO = Path(__file__).resolve().parent.parent
ASSETS = REPO / "assets"


def squircle_icon(size: int) -> Image.Image:
    """Full-bleed cream squircle with the coral record dot, centered."""
    big = size * SS
    img = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)
    draw.rounded_rectangle(
        [0, 0, big - 1, big - 1], radius=round(big * 0.225), fill=CREAM
    )
    dot = round(big * 0.185)  # dot diameter ~37% of the canvas
    center = big // 2
    draw.ellipse(
        [center - dot, center - dot, center + dot, center + dot], fill=CORAL
    )
    return img.resize((size, size), Image.LANCZOS)


def tray_icon(size: int) -> Image.Image:
    """Bare dot on transparency; proportionally bigger for 16px trays."""
    big = size * SS
    img = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)
    dot = round(big * 0.30)  # dot diameter ~60% of the canvas
    center = big // 2
    draw.ellipse(
        [center - dot, center - dot, center + dot, center + dot], fill=CORAL
    )
    return img.resize((size, size), Image.LANCZOS)


def pack_icns(entries: dict[int, bytes]) -> bytes:
    """Minimal ICNS container: PNG-encoded entries, modern types only."""
    codes = {16: "icp4", 32: "icp5", 128: "ic07", 256: "ic08"}
    body = b""
    for size in sorted(entries):
        if size not in codes:
            continue
        data = entries[size]
        body += codes[size].encode("ascii") + struct.pack(">I", len(data) + 8) + data
    return b"icns" + struct.pack(">I", len(body) + 8) + body


def png_bytes(img: Image.Image) -> bytes:
    buffer = BytesIO()
    img.save(buffer, format="PNG")
    return buffer.getvalue()


def preview(app: Image.Image, tray_light: Image.Image) -> Image.Image:
    """A review sheet: the app icon and the tray dot on light and dark."""
    margin, cell = 24, 96
    width = margin * 2 + cell * 3
    height = margin * 2 + cell
    sheet = Image.new("RGBA", (width, height), (255, 255, 255, 255))
    sheet.paste(app.resize((cell, cell), Image.LANCZOS), (margin, margin), app.resize((cell, cell), Image.LANCZOS))
    dark = Image.new("RGBA", (cell, cell), (46, 46, 48, 255))
    sheet.paste(dark, (margin + cell, margin))
    tray_dark = tray_light.resize((cell // 2, cell // 2), Image.LANCZOS)
    position = (margin + cell + cell // 4, margin + cell // 4)
    sheet.paste(tray_dark, position, tray_dark)
    sheet.paste(tray_light.resize((cell // 2, cell // 2), Image.LANCZOS), (margin + cell * 2 + cell // 4, margin + cell // 4), tray_light.resize((cell // 2, cell // 2), Image.LANCZOS))
    return sheet


def main() -> None:
    ASSETS.mkdir(exist_ok=True)

    app_icons = {size: squircle_icon(size) for size in SIZES}
    for size, img in app_icons.items():
        img.save(ASSETS / f"icon-{size}.png")

    # Multi-size Windows icon (Pillow resamples from the 256px source).
    app_icons[256].save(ASSETS / "icon.ico", sizes=[(s, s) for s in SIZES])

    # macOS icon (PNG entries in a minimal ICNS container).
    icns_pngs = {
        size: png_bytes(app_icons[size]) for size in ICNS_SIZES if size in app_icons
    }
    (ASSETS / "icon.icns").write_bytes(pack_icns(icns_pngs))

    tray = tray_icon(32)
    tray.save(ASSETS / "tray-32.png")

    preview(app_icons[256], tray).save(REPO / "tools" / "preview.png")
    print(f"icons written to {ASSETS}")


if __name__ == "__main__":
    main()
