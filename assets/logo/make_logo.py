#!/usr/bin/env python3
"""Generates every RAMForge logo asset from one geometry definition.

Outputs (next to this script, in ./out):
  ramforge-icon.svg            square app icon (vector master)
  ramforge-logo-dark.svg       icon + wordmark for dark backgrounds
  ramforge-logo-light.svg      icon + wordmark for light backgrounds
  ramforge-icon-<n>.png        16 ... 1024 px
  ramforge.ico                 Windows EXE icon (16, 24, 32, 48, 64, 128, 256)
  favicon.ico                  website favicon (16, 32, 48)
  apple-touch-icon.png         180 px
  og-image.png                 1200x630 social preview

Requires only Pillow. Run:  python make_logo.py
The logo is part of RAMForge and licensed like the project (GPL-3.0-or-later);
see assets/logo/README.md for trademark-style usage notes.
"""
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont

OUT = Path(__file__).with_name("out")
G = 512  # design grid

BG_TOP, BG_BOTTOM = (35, 40, 56), (11, 13, 19)
HOT = [(0.0, (255, 181, 71)), (0.55, (255, 107, 26)), (1.0, (217, 58, 43))]
SPARK = (255, 209, 102)
CHIP = (42, 22, 16)
PIN = (255, 224, 138)

MODULE = [(78, 236), (434, 236), (448, 250), (448, 356), (308, 356), (308, 338),
          (292, 338), (292, 356), (64, 356), (64, 250)]
CHIPS = [(86 + i * 90, 256, 70, 62) for i in range(4)]
PINS = [x for x in range(78, 440, 16) if not 284 <= x <= 308]
SPARKS = [(362, 150, 58), (424, 98, 26), (300, 92, 18)]


def star(cx, cy, r, k=0.26):
    i = r * k
    return [(cx, cy - r), (cx + i, cy - i), (cx + r, cy), (cx + i, cy + i),
            (cx, cy + r), (cx - i, cy + i), (cx - r, cy), (cx - i, cy - i)]


# ---------------------------------------------------------------- SVG
def svg_defs():
    stops = "".join(f'<stop offset="{o}" stop-color="rgb{c}"/>' for o, c in HOT)
    return f"""<defs>
  <linearGradient id="rf-bg" x1="0" y1="0" x2="1" y2="1">
    <stop offset="0" stop-color="rgb{BG_TOP}"/><stop offset="1" stop-color="rgb{BG_BOTTOM}"/>
  </linearGradient>
  <linearGradient id="rf-hot" x1="0" y1="0" x2="0" y2="1">{stops}</linearGradient>
  <radialGradient id="rf-glow" cx="0.5" cy="0.5" r="0.5">
    <stop offset="0" stop-color="#ff6b1a" stop-opacity="0.55"/><stop offset="1" stop-color="#ff6b1a" stop-opacity="0"/>
  </radialGradient>
  <radialGradient id="rf-spark" cx="0.5" cy="0.5" r="0.5">
    <stop offset="0" stop-color="#ffffff"/><stop offset="0.45" stop-color="rgb{SPARK}"/><stop offset="1" stop-color="#ff9f1c"/>
  </radialGradient>
</defs>"""


def pts(p):
    return " ".join(f"{x:g},{y:g}" for x, y in p)


def svg_mark():
    chips = "".join(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="6" fill="rgb{CHIP}" fill-opacity="0.82"/>' for x, y, w, h in CHIPS)
    pins = "".join(f'<rect x="{x}" y="330" width="8" height="20" rx="2" fill="rgb{PIN}" fill-opacity="0.9"/>' for x in PINS)
    sparks = "".join(f'<polygon points="{pts(star(*s))}" fill="url(#rf-spark)"/>' for s in SPARKS)
    return (f'<ellipse cx="256" cy="300" rx="230" ry="120" fill="url(#rf-glow)"/>'
            f'<polygon points="{pts(MODULE)}" fill="url(#rf-hot)"/>{chips}{pins}{sparks}')


def svg_icon():
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {G} {G}" role="img" aria-label="RAMForge">'
            f'{svg_defs()}<rect x="16" y="16" width="480" height="480" rx="104" fill="url(#rf-bg)"/>'
            f'<rect x="17" y="17" width="478" height="478" rx="103" fill="none" stroke="#ffffff" stroke-opacity="0.08" stroke-width="2"/>'
            f'{svg_mark()}</svg>\n')


def svg_wordmark(text_color):
    font = "Inter, 'Segoe UI', system-ui, -apple-system, Roboto, Arial, sans-serif"
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1720 {G}" role="img" aria-label="RAMForge">'
            f'{svg_defs()}<rect x="16" y="16" width="480" height="480" rx="104" fill="url(#rf-bg)"/>{svg_mark()}'
            f'<text x="560" y="335" font-family="{font}" font-size="236" font-weight="800" letter-spacing="-6">'
            f'<tspan fill="{text_color}">RAM</tspan><tspan fill="url(#rf-hot)">Forge</tspan></text></svg>\n')


# ---------------------------------------------------------------- raster
def gradient(size, stops, vertical=True):
    w, h = size
    n = h if vertical else w
    line = Image.new("RGB", (1, n) if vertical else (n, 1))
    for i in range(n):
        t = i / max(n - 1, 1)
        for (o0, c0), (o1, c1) in zip(stops, stops[1:]):
            if o0 <= t <= o1:
                f = (t - o0) / (o1 - o0)
                c = tuple(round(a + (b - a) * f) for a, b in zip(c0, c1))
                line.putpixel((0, i) if vertical else (i, 0), c)
                break
    return line.resize((w, h))


def render(px):
    s = 4 * px / G  # supersampling factor
    S = round(G * s)
    sc = lambda p: [(x * s, y * s) for x, y in p]
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))

    # background tile with diagonal gradient
    diag = gradient((S * 2, S * 2), [(0, BG_TOP), (1, BG_BOTTOM)]).rotate(45, resample=Image.BICUBIC)
    diag = diag.crop((S // 2, S // 2, S // 2 + S, S // 2 + S))
    m = Image.new("L", (S, S), 0)
    ImageDraw.Draw(m).rounded_rectangle([16 * s, 16 * s, 496 * s, 496 * s], radius=104 * s, fill=255)
    img.paste(diag, (0, 0), m)

    # glow
    glow = Image.new("L", (S, S), 0)
    gd = ImageDraw.Draw(glow)
    for i in range(40, 0, -1):
        f = i / 40
        gd.ellipse([(256 - 230 * f) * s, (300 - 120 * f) * s, (256 + 230 * f) * s, (300 + 120 * f) * s],
                   fill=round(140 * (1 - f)))
    img.paste((255, 107, 26), (0, 0), Image.composite(glow, Image.new("L", (S, S), 0), m))

    # module with notch
    hot = gradient((S, round(120 * s)), HOT)
    mm = Image.new("L", (S, S), 0)
    ImageDraw.Draw(mm).polygon(sc(MODULE), fill=255)
    layer = Image.new("RGB", (S, S))
    layer.paste(hot, (0, round(236 * s)))
    img.paste(layer, (0, 0), mm)

    d = ImageDraw.Draw(img)
    for x, y, w, h in CHIPS:
        d.rounded_rectangle([x * s, y * s, (x + w) * s, (y + h) * s], radius=6 * s, fill=CHIP + (210,))
    for x in PINS:
        d.rounded_rectangle([x * s, 330 * s, (x + 8) * s, 350 * s], radius=2 * s, fill=PIN + (230,))
    for cx, cy, r in SPARKS:
        d.polygon(sc(star(cx, cy, r)), fill=SPARK)
        d.polygon(sc(star(cx, cy, r * 0.45)), fill=(255, 255, 255))
    return img.resize((px, px), Image.LANCZOS)


def og_image(icon):
    W, H = 1200, 630
    img = gradient((W, H), [(0, (22, 25, 35)), (1, (9, 10, 15))]).convert("RGBA")
    img.alpha_composite(icon.resize((320, 320), Image.LANCZOS), (70, 155))
    d = ImageDraw.Draw(img)
    try:
        big = ImageFont.truetype("C:/Windows/Fonts/segoeuib.ttf", 112)
        small = ImageFont.truetype("C:/Windows/Fonts/segoeui.ttf", 34)
    except OSError:
        big = small = ImageFont.load_default()
    d.text((430, 215), "RAM", font=big, fill=(240, 242, 247))
    x = 430 + d.textlength("RAM", font=big)
    d.text((x, 215), "Forge", font=big, fill=(255, 122, 32))
    d.text((434, 365), "Open-source RAM test, benchmark & sensors", font=small, fill=(170, 176, 190))
    d.text((434, 412), "Windows · bootable USB · ramforge.org", font=small, fill=(120, 126, 140))
    return img.convert("RGB")


def main():
    OUT.mkdir(exist_ok=True)
    (OUT / "ramforge-icon.svg").write_text(svg_icon(), encoding="utf-8")
    (OUT / "ramforge-logo-dark.svg").write_text(svg_wordmark("#f0f2f7"), encoding="utf-8")
    (OUT / "ramforge-logo-light.svg").write_text(svg_wordmark("#141821"), encoding="utf-8")
    sizes = [16, 24, 32, 48, 64, 128, 180, 256, 512, 1024]
    imgs = {n: render(n) for n in sizes}
    for n, im in imgs.items():
        if n != 180:
            im.save(OUT / f"ramforge-icon-{n}.png", optimize=True)
    imgs[180].save(OUT / "apple-touch-icon.png", optimize=True)
    imgs[256].save(OUT / "ramforge.ico", sizes=[(n, n) for n in (16, 24, 32, 48, 64, 128, 256)])
    imgs[48].save(OUT / "favicon.ico", sizes=[(16, 16), (32, 32), (48, 48)])
    og_image(imgs[1024]).save(OUT / "og-image.png", optimize=True)
    print("written to", OUT)


if __name__ == "__main__":
    main()
