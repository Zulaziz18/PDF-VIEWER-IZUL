#!/usr/bin/env python3
"""Pictures with a known subject, for measuring background removal (Phase 7).

Each picture is drawn here with its exact mask (255 = subject), so the result
can be scored: a salient object on a textured, uneven background — the case
the model is for — and the two pictures people put into PDFs most often:
a photographed signature and a stamp, on off-white paper with shading.
Nothing is taken from anywhere else.

    python3 tools/background-proof/make_pictures.py <out-dir>
"""
import json
import math
import random
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter


def textured(w, h, seed, base, spread):
    rnd = random.Random(seed)
    img = Image.new("RGB", (w, h), base)
    d = ImageDraw.Draw(img)
    for _ in range(400):
        x, y = rnd.randrange(w), rnd.randrange(h)
        r = rnd.randint(4, 40)
        c = tuple(max(0, min(255, b + rnd.randint(-spread, spread))) for b in base)
        d.ellipse((x - r, y - r, x + r, y + r), fill=c)
    return img.filter(ImageFilter.GaussianBlur(6))


def shade(img, strength):
    """Uneven light, as a phone photo of paper has."""
    w, h = img.size
    px = img.load()
    for y in range(h):
        for x in range(w):
            f = 1 - strength * ((x / w) ** 2 + (y / h) * 0.5)
            r, g, b = px[x, y]
            px[x, y] = (int(r * f), int(g * f), int(b * f))
    return img


def mug(seed):
    w, h = 480, 360
    img = textured(w, h, seed, (120, 150, 110), 40)
    mask = Image.new("L", (w, h), 0)
    for canvas, fill, handle in ((img, (200, 60, 40), (150, 40, 30)), (mask, 255, 255)):
        d = ImageDraw.Draw(canvas)
        d.rounded_rectangle((150, 90, 300, 290), 18, fill=fill)
        d.ellipse((280, 140, 360, 240), outline=handle, width=18)
    d = ImageDraw.Draw(img)
    d.rectangle((170, 110, 190, 270), fill=(230, 110, 90))  # a highlight
    return img, mask


def fruit(seed):
    w, h = 400, 400
    img = textured(w, h, seed, (70, 60, 90), 30)
    mask = Image.new("L", (w, h), 0)
    ImageDraw.Draw(img).ellipse((110, 120, 290, 300), fill=(240, 180, 30))
    ImageDraw.Draw(img).ellipse((150, 150, 200, 200), fill=(255, 225, 120))
    ImageDraw.Draw(img).rectangle((195, 80, 205, 125), fill=(90, 60, 20))
    ImageDraw.Draw(mask).ellipse((110, 120, 290, 300), fill=255)
    ImageDraw.Draw(mask).rectangle((195, 80, 205, 125), fill=255)
    return img, mask


def signature(seed):
    rnd = random.Random(seed)
    w, h = 520, 220
    img = shade(Image.new("RGB", (w, h), (244, 240, 228)), 0.18)
    mask = Image.new("L", (w, h), 0)
    pts = []
    for i in range(160):
        t = i / 159
        x = 40 + t * 440
        y = 110 + 45 * math.sin(t * 11 + seed) * math.sin(t * 3.1) + rnd.uniform(-3, 3)
        pts.append((x, y))
    for canvas, fill in ((img, (25, 35, 110)), (mask, 255)):
        ImageDraw.Draw(canvas).line(pts, fill=fill, width=5, joint="curve")
    return img, mask


def stamp(seed):
    w, h = 360, 360
    img = shade(Image.new("RGB", (w, h), (246, 243, 236)), 0.22)
    mask = Image.new("L", (w, h), 0)
    for canvas, fill in ((img, (170, 30, 40)), (mask, 255)):
        d = ImageDraw.Draw(canvas)
        d.ellipse((50, 50, 310, 310), outline=fill, width=12)
        d.ellipse((95, 95, 265, 265), outline=fill, width=6)
        d.rectangle((110, 165, 250, 195), fill=fill)
    return img, mask


def white_product(seed):
    """The case that can go wrong: a white object photographed on white."""
    w, h = 420, 360
    img = shade(Image.new("RGB", (w, h), (238, 238, 236)), 0.12)
    mask = Image.new("L", (w, h), 0)
    d = ImageDraw.Draw(img)
    d.ellipse((140, 270, 300, 300), fill=(205, 205, 203))  # its shadow
    d.rounded_rectangle((150, 80, 280, 280), 20, fill=(250, 250, 250), outline=(170, 170, 172), width=3)
    d.rectangle((165, 100, 185, 260), fill=(255, 255, 255))
    d.rectangle((245, 90, 275, 270), fill=(214, 214, 216))
    ImageDraw.Draw(mask).rounded_rectangle((150, 80, 280, 280), 20, fill=255)
    return img, mask


CASES = [("cangkir", mug, "objek di atas latar bertekstur"), ("buah", fruit, "objek di atas latar gelap bertekstur"),
         ("tanda-tangan", signature, "tanda tangan difoto di atas kertas"), ("stempel", stamp, "stempel di atas kertas"),
         ("produk-putih", white_product, "benda putih di atas latar putih (kasus yang bisa salah)")]


def main():
    out = Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=True)
    meta = []
    for name, fn, what in CASES:
        img, mask = fn(len(name))
        img.save(out / f"{name}.png")
        mask.save(out / f"{name}.truth.png")
        meta.append({"name": name, "what": what})
    (out / "cases.json").write_text(json.dumps(meta, ensure_ascii=False))


if __name__ == "__main__":
    main()
