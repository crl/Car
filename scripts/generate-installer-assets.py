"""Generate NSIS branding. Run from any directory with Python + Pillow 12.3.0.

Uses the existing app icon and Windows Microsoft YaHei Bold font by default.
Pass --font to use the same font file on another machine. Generated 24-bit BMPs
are checked in, so building the app does not require Python, Pillow or fonts.
"""

import argparse
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont


ROOT = Path(__file__).resolve().parents[1]
SCALE = 4


def generate(font_path):
    output = ROOT / "src-tauri" / "installer"
    output.mkdir(exist_ok=True)
    icon = Image.open(ROOT / "src-tauri" / "app-icon.png").convert("RGBA")
    # The source icon has deliberate outer padding; crop it for branding layouts.
    icon = icon.crop(icon.getbbox())

    def font(size):
        return ImageFont.truetype(str(font_path), size * SCALE)

    def paste_icon(canvas, x, y, size):
        resized = icon.resize((size * SCALE, size * SCALE), Image.Resampling.LANCZOS)
        canvas.paste(resized, (x * SCALE, y * SCALE), resized)

    sidebar = Image.new("RGB", (164 * SCALE, 314 * SCALE))
    draw = ImageDraw.Draw(sidebar)
    for y in range(sidebar.height):
        t = y / (sidebar.height - 1)
        color = tuple(round(a + (b - a) * t) for a, b in zip((247, 242, 255), (225, 211, 248)))
        draw.line((0, y, sidebar.width, y), fill=color)
    draw.ellipse((65*SCALE, -65*SCALE, 260*SCALE, 130*SCALE), fill="#eee4fc")
    draw.ellipse((-105*SCALE, 225*SCALE, 130*SCALE, 455*SCALE), fill="#d9c6f1")
    # A soft route and destination echo the transport identity without extra copy.
    route = [(35, 255), (55, 240), (105, 240), (129, 218)]
    draw.line([(x*SCALE, y*SCALE) for x, y in route], fill="#b69bdc", width=2*SCALE, joint="curve")
    draw.ellipse((124*SCALE, 213*SCALE, 134*SCALE, 223*SCALE), fill="#9873cc")
    draw.ellipse((127*SCALE, 216*SCALE, 131*SCALE, 220*SCALE), fill="#f7f2ff")
    paste_icon(sidebar, 40, 64, 84)
    draw.text((82*SCALE, 168*SCALE), "运管家", font=font(23), fill="#654298", anchor="mt")
    sidebar.resize((164, 314), Image.Resampling.LANCZOS).save(output / "sidebar.bmp")

    header = Image.new("RGB", (150*SCALE, 57*SCALE), "white")
    paste_icon(header, 105, 10, 37)
    draw = ImageDraw.Draw(header)
    draw.text((94*SCALE, 28*SCALE), "运管家", font=font(16), fill="#7653ab", anchor="rm")
    header.resize((150, 57), Image.Resampling.LANCZOS).save(output / "header.bmp")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--font", type=Path, default=Path("C:/Windows/Fonts/msyhbd.ttc"))
    generate(parser.parse_args().font)
