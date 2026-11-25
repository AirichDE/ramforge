#!/usr/bin/env python3
"""Boot menu background for the RAMForge boot edition (GRUB and ISOLINUX).

Writes splash.png (1024x768, GRUB), splash-640.png and splash-800.png (ISOLINUX).
The top third carries the branding; the menu is drawn by the boot loader in
the lower part. Requires Pillow; run on Windows (uses Segoe UI for the text,
like assets/logo/make_logo.py).
"""
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont

HERE = Path(__file__).parent
LOGO = HERE.parents[2] / "assets" / "logo" / "out" / "ramforge-icon-256.png"
VERSION = "0.3"
TOP, BOTTOM = (35, 40, 56), (11, 13, 19)
ORANGE = (242, 107, 42)


def render(w: int, h: int, footer: bool = True) -> Image.Image:
    img = Image.new("RGB", (w, h))
    px = img.load()
    for y in range(h):
        t = y / (h - 1)
        c = tuple(int(TOP[i] * (1 - t) + BOTTOM[i] * t) for i in range(3))
        for x in range(w):
            px[x, y] = c
    s = w / 1024
    logo = Image.open(LOGO).convert("RGBA").resize((int(120 * s), int(120 * s)), Image.LANCZOS)
    img.paste(logo, (int(60 * s), int(48 * s)), logo)
    d = ImageDraw.Draw(img)
    big = ImageFont.truetype("C:/Windows/Fonts/segoeuib.ttf", int(64 * s))
    mid = ImageFont.truetype("C:/Windows/Fonts/segoeui.ttf", int(24 * s))
    small = ImageFont.truetype("C:/Windows/Fonts/segoeui.ttf", int(18 * s))
    x0 = int(200 * s)
    d.text((x0, int(52 * s)), "RAM", font=big, fill=ORANGE)
    rw = d.textlength("RAM", font=big)
    d.text((x0 + rw, int(52 * s)), "Forge", font=big, fill=(240, 242, 246))
    d.text((x0 + rw + d.textlength("Forge", font=big) + 16 * s, int(84 * s)), VERSION, font=mid, fill=(154, 163, 176))
    d.text((x0, int(130 * s)), "RAM-Test · Benchmark · Hardware-Monitor", font=mid, fill=(200, 205, 214))
    d.line([(int(60 * s), int(190 * s)), (w - int(60 * s), int(190 * s))], fill=(60, 66, 80), width=max(1, int(2 * s)))
    foot = "© 2026 Daniel Airich  ·  ramforge.org  ·  freie Software (GPL-3.0)"
    if footer:
        fw = d.textlength(foot, font=small)
        d.text(((w - fw) / 2, h - int(40 * s)), foot, font=small, fill=(154, 163, 176))
    else:
        d.text((x0, int(160 * s)), "© 2026 Daniel Airich · ramforge.org", font=small, fill=(154, 163, 176))
    return img


if __name__ == "__main__":
    render(1024, 768).save(HERE / "splash.png", optimize=True)
    render(640, 480, footer=False).save(HERE / "splash-640.png", optimize=True)
    render(800, 600, footer=False).save(HERE / "splash-800.png", optimize=True)
    print("ok")
