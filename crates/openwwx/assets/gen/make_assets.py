#!/usr/bin/env python3
"""Generates OpenWWX's original UI images (Pillow + NumPy).

    python3 make_assets.py <fonts dir> <out dir>

The fonts dir needs Anton-Regular.ttf and AlfaSlabOne-Regular.ttf (both SIL OFL, from
github.com/google/fonts). Everything is drawn here from shapes and noise, with no outside art.
"""

import math
import random
import sys
from pathlib import Path

import numpy as np
from PIL import Image, ImageChops, ImageDraw, ImageFilter, ImageFont

FONTS = Path(sys.argv[1])
OUT = Path(sys.argv[2])
OUT.mkdir(parents=True, exist_ok=True)
(OUT / "iconos").mkdir(exist_ok=True)
rng = random.Random(7)
nrng = np.random.default_rng(7)


def font(name, size):
    return ImageFont.truetype(str(FONTS / name), size)


def noise(w, h, scale=1.0, blur=0):
    n = (nrng.random((h, w)) * 255).astype(np.uint8)
    img = Image.fromarray(n, "L")
    if blur:
        img = img.filter(ImageFilter.GaussianBlur(blur))
    return img


def vgrad(w, h, top, bottom):
    t = np.linspace(0, 1, h)[:, None, None]
    a = np.array(top, float)[None, None, :]
    b = np.array(bottom, float)[None, None, :]
    arr = a + (b - a) * t
    arr = np.repeat(arr, w, axis=1)
    return Image.fromarray(arr.clip(0, 255).astype(np.uint8), "RGB" if len(top) == 3 else "RGBA")


def glow(size, center, radius, color, strength=1.0):
    w, h = size
    y, x = np.mgrid[0:h, 0:w]
    d = np.sqrt((x - center[0]) ** 2 + (y - center[1]) ** 2) / radius
    a = np.clip(1 - d, 0, 1) ** 2 * strength
    arr = np.zeros((h, w, 3))
    for i in range(3):
        arr[..., i] = color[i] * a
    return arr


# ---------------------------------------------------------------- arena background
def arena():
    W, H = 1920, 1080
    base = np.array(vgrad(W, H, (10, 8, 14), (4, 3, 6)), float)
    # red stage wash from the right and the floor
    base += glow((W, H), (1450, 560), 900, (120, 10, 14), 0.9)
    base += glow((W, H), (300, 300), 700, (70, 8, 12), 0.6)
    base += glow((W, H), (960, 1150), 1000, (90, 12, 16), 0.7)
    img = Image.fromarray(base.clip(0, 255).astype(np.uint8))

    # light beams from the truss
    beams = Image.new("RGB", (W, H))
    bd = ImageDraw.Draw(beams)
    for i in range(9):
        x = 120 + i * 210 + rng.randint(-40, 40)
        tx = x + rng.randint(-500, 500)
        spread = rng.randint(90, 200)
        col = (150, 30, 30) if i % 3 else (130, 130, 150)
        bd.polygon([(x - 6, 70), (x + 6, 70), (tx + spread, H), (tx - spread, H)], fill=col)
    beams = beams.filter(ImageFilter.GaussianBlur(40))
    img = ImageChops.add(img, Image.eval(beams, lambda v: v // 3))

    d = ImageDraw.Draw(img)
    # truss along the top
    for y in (40, 78):
        d.rectangle([0, y, W, y + 6], fill=(28, 26, 30))
    for x in range(0, W, 38):
        d.line([(x, 46), (x + 38, 78)], fill=(36, 34, 38), width=3)
        d.line([(x + 38, 46), (x, 78)], fill=(36, 34, 38), width=3)
    # spotlights hanging on the truss
    lamps = Image.new("RGB", (W, H))
    ld = ImageDraw.Draw(lamps)
    for i in range(16):
        x = 60 + i * 120 + rng.randint(-10, 10)
        warm = i % 4 == 0
        c = (255, 240, 220) if warm else (255, 70, 60)
        ld.ellipse([x - 9, 88, x + 9, 106], fill=c)
    halo = lamps.filter(ImageFilter.GaussianBlur(18))
    img = ImageChops.add(img, halo)
    img = ImageChops.add(img, lamps)
    d = ImageDraw.Draw(img)

    # big screens on the sides, blank and glowing
    for x0 in (60, W - 460):
        d.rectangle([x0, 150, x0 + 400, 380], fill=(40, 6, 10), outline=(70, 70, 76), width=6)
        for yy in range(156, 376, 4):
            d.line([(x0 + 6, yy), (x0 + 394, yy)], fill=(55, 10, 14))

    # crowd: rows of blurred heads with camera flashes
    crowd = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    cd = ImageDraw.Draw(crowd)
    for row in range(14):
        y = 560 + row * 38
        for _ in range(130):
            x = rng.randint(-20, W + 20)
            r = rng.randint(10, 16) + row
            g = rng.randint(6, 18)
            cd.ellipse([x - r, y - r, x + r, y + r * 2], fill=(g, g - 2 if g > 2 else 0, g + 2, 255))
    crowd = crowd.filter(ImageFilter.GaussianBlur(3))
    img.paste(crowd, (0, 0), crowd)
    flashes = Image.new("RGB", (W, H))
    fd = ImageDraw.Draw(flashes)
    for _ in range(160):
        x, y = rng.randint(0, W), rng.randint(520, H)
        r = rng.randint(2, 7)
        c = rng.choice([(255, 255, 255), (255, 200, 160), (255, 80, 70), (120, 160, 255)])
        fd.ellipse([x - r, y - r, x + r, y + r], fill=c)
    img = ImageChops.add(img, flashes.filter(ImageFilter.GaussianBlur(6)))
    img = ImageChops.add(img, Image.eval(flashes.filter(ImageFilter.GaussianBlur(1)), lambda v: v // 2))

    # vignette
    y, x = np.mgrid[0:H, 0:W]
    v = 1 - 0.75 * (((x - W / 2) / (W / 1.3)) ** 2 + ((y - H / 2) / (H / 1.2)) ** 2)
    arr = np.array(img, float) * np.clip(v, 0.15, 1)[..., None]
    # film grain
    arr += (nrng.random((H, W, 1)) - 0.5) * 10
    img = Image.fromarray(arr.clip(0, 255).astype(np.uint8))
    img.save(OUT / "arena.jpg", quality=84, optimize=True)


# ---------------------------------------------------------------- metal and red fills
def brushed(w, h, dark, light):
    n = noise(w, h).resize((w, max(1, h // 1)))
    streak = n.filter(ImageFilter.BoxBlur(0)).resize((max(1, w // 40), h)).resize((w, h))
    g = np.array(vgrad(w, h, light, dark), float)
    s = np.array(streak, float)[..., None] / 255
    return Image.fromarray((g * (0.85 + 0.3 * s)).clip(0, 255).astype(np.uint8))


def scratches(img, count, color, alpha):
    over = Image.new("RGBA", img.size, (0, 0, 0, 0))
    d = ImageDraw.Draw(over)
    w, h = img.size
    for _ in range(count):
        x, y = rng.randint(0, w), rng.randint(0, h)
        ang = rng.uniform(-0.5, 0.5)
        ln = rng.randint(8, 60)
        d.line([(x, y), (x + ln * math.cos(ang), y + ln * math.sin(ang))], fill=color + (alpha,), width=1)
    return Image.alpha_composite(img.convert("RGBA"), over)


def slanted_mask(w, h, slant, inset=0):
    m = Image.new("L", (w, h), 0)
    ImageDraw.Draw(m).polygon(
        [(slant + inset, inset), (w - inset - 1, inset), (w - slant - inset - 1, h - inset - 1), (inset, h - inset - 1)],
        fill=255,
    )
    return m


def button(hover):
    W, H, S = 640, 104, 34
    chrome = brushed(W, H, (60, 60, 66), (225, 225, 232))
    if hover:
        red = vgrad(W, H, (245, 70, 60), (150, 10, 12))
    else:
        red = vgrad(W, H, (205, 30, 28), (105, 6, 8))
    red = scratches(red, 140, (255, 200, 200), 35)
    # a soft gloss band on the top half
    gloss = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    ImageDraw.Draw(gloss).rectangle([0, 8, W, H // 2 - 4], fill=(255, 255, 255, 34 if hover else 24))
    red = Image.alpha_composite(red, gloss.filter(ImageFilter.GaussianBlur(6)))
    out = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    out.paste(chrome, (0, 0), slanted_mask(W, H, S))
    out.paste(Image.new("RGB", (W, H), (20, 18, 20)), (0, 0), slanted_mask(W, H, S, 5))
    out.paste(red, (0, 0), slanted_mask(W, H, S, 8))
    # dark notch on the left for the icon
    notch = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    ImageDraw.Draw(notch).polygon([(S + 8, 8), (S + 96, 8), (96 - 0, H - 9), (8, H - 9)], fill=(25, 4, 6, 120))
    out = Image.alpha_composite(out, Image.composite(notch, Image.new("RGBA", (W, H)), slanted_mask(W, H, S, 8)))
    if hover:
        g = out.filter(ImageFilter.GaussianBlur(10))
        a = g.getchannel("A").point(lambda v: v // 3)
        halo = Image.new("RGBA", (W, H), (255, 60, 40, 0))
        halo.putalpha(a)
        out = Image.alpha_composite(halo, out)
    out.save(OUT / ("boton-hover.png" if hover else "boton.png"), optimize=True)


def bar():
    W, H, S = 640, 60, 20
    chrome = brushed(W, H, (40, 40, 44), (150, 150, 158))
    dark = vgrad(W, H, (34, 30, 34), (14, 12, 14))
    dark = scratches(dark, 80, (255, 255, 255), 14)
    out = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    out.paste(chrome, (0, 0), slanted_mask(W, H, S))
    out.paste(dark, (0, 0), slanted_mask(W, H, S, 3))
    # red accent on the left
    acc = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    ImageDraw.Draw(acc).polygon([(S + 3, 3), (S + 70, 3), (70, H - 4), (3, H - 4)], fill=(150, 14, 16, 255))
    out = Image.alpha_composite(out, Image.composite(acc, Image.new("RGBA", (W, H)), slanted_mask(W, H, S, 3)))
    out.save(OUT / "barra.png", optimize=True)


def frame(name, light, dark, glow_col):
    W, H, B = 330, 450, 22
    metal = brushed(W, H, dark, light)
    metal = scratches(metal, 120, (255, 255, 255), 40)
    out = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    ring = Image.new("L", (W, H), 0)
    rd = ImageDraw.Draw(ring)
    rd.rounded_rectangle([0, 0, W - 1, H - 1], 14, fill=255)
    rd.rounded_rectangle([B, B, W - B - 1, H - B - 1], 6, fill=0)
    out.paste(metal, (0, 0), ring)
    d = ImageDraw.Draw(out)
    # bevels
    d.rounded_rectangle([2, 2, W - 3, H - 3], 13, outline=(255, 255, 255, 90), width=2)
    d.rounded_rectangle([B - 3, B - 3, W - B + 2, H - B + 2], 7, outline=(0, 0, 0, 200), width=3)
    d.rounded_rectangle([B - 6, B - 6, W - B + 5, H - B + 5], 8, outline=glow_col + (200,), width=2)
    # corner bolts
    for cx, cy in [(B // 2 + 1, B // 2 + 1), (W - B // 2 - 2, B // 2 + 1), (B // 2 + 1, H - B // 2 - 2),
                   (W - B // 2 - 2, H - B // 2 - 2)]:
        d.ellipse([cx - 6, cy - 6, cx + 6, cy + 6], fill=(40, 40, 44, 255), outline=(230, 230, 230, 255))
        d.line([(cx - 3, cy), (cx + 3, cy)], fill=(230, 230, 230, 255), width=2)
    # outer glow
    a = out.getchannel("A").filter(ImageFilter.GaussianBlur(8)).point(lambda v: v // 2)
    halo = Image.new("RGBA", (W, H), glow_col + (0,))
    halo.putalpha(a)
    pad = Image.new("RGBA", (W + 40, H + 40), (0, 0, 0, 0))
    big_halo = Image.new("RGBA", (W + 40, H + 40), glow_col + (0,))
    ha = Image.new("L", (W + 40, H + 40), 0)
    ha.paste(out.getchannel("A"), (20, 20))
    big_halo.putalpha(ha.filter(ImageFilter.GaussianBlur(10)).point(lambda v: int(v * 0.6)))
    pad = Image.alpha_composite(pad, big_halo)
    pad.alpha_composite(out, (20, 20))
    pad.save(OUT / name, optimize=True)


# ---------------------------------------------------------------- logo
def logo():
    W, H = 1100, 300
    f = font("AlfaSlabOne-Regular.ttf", 190)
    text = "OPENWWX"
    mask = Image.new("L", (W, H), 0)
    md = ImageDraw.Draw(mask)
    bbox = md.textbbox((0, 0), text, font=f)
    tw, th = bbox[2] - bbox[0], bbox[3] - bbox[1]
    # condense horizontally a little
    tmp = Image.new("L", (tw + 40, th + 60), 0)
    ImageDraw.Draw(tmp).text((20 - bbox[0], 30 - bbox[1]), text, font=f, fill=255)
    tmp = tmp.resize((int(tmp.width * 0.9), tmp.height))
    # slight upward skew so it reads as dynamic
    tmp = tmp.transform(tmp.size, Image.AFFINE, (1, 0.12, -0.12 * tmp.height / 2, 0, 1, 0), Image.BICUBIC)
    mask.paste(tmp, ((W - tmp.width) // 2, (H - tmp.height) // 2 - 10))

    # distress: knock random specks out of the letters
    specks = noise(W, H, blur=1.2).point(lambda v: 255 if v > 150 else 0)
    specks = ImageChops.multiply(specks, noise(W, H, blur=6).point(lambda v: 255 if v > 132 else 0))
    inner = ImageChops.subtract(mask, specks)

    red = vgrad(W, H, (255, 60, 46), (140, 0, 6)).convert("RGBA")
    red = scratches(red, 400, (60, 0, 0), 120)
    outline_w = mask.filter(ImageFilter.MaxFilter(15))
    outline_d = mask.filter(ImageFilter.MaxFilter(23))
    shadow = outline_d.filter(ImageFilter.GaussianBlur(10))

    out = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    sh = Image.new("RGBA", (W, H), (0, 0, 0, 255))
    out.paste(sh, (6, 10), shadow)
    out.paste(Image.new("RGBA", (W, H), (12, 10, 12, 255)), (0, 0), outline_d)
    out.paste(brushed(W, H, (110, 110, 118), (245, 245, 250)).convert("RGBA"), (0, 0), outline_w)
    out.paste(Image.new("RGBA", (W, H), (30, 0, 2, 255)), (0, 0), mask.filter(ImageFilter.MaxFilter(5)))
    out.paste(red, (0, 0), inner)
    # top highlight on the letters
    hl = ImageChops.subtract(inner, ImageChops.offset(inner, 0, 6)).filter(ImageFilter.GaussianBlur(1))
    out.paste(Image.new("RGBA", (W, H), (255, 190, 170, 255)), (0, 0), hl.point(lambda v: v // 2))
    out = out.crop(out.getbbox())
    out.save(OUT / "logo.png", optimize=True)


# ---------------------------------------------------------------- ring pieces
def canvas():
    W = 256
    base = np.full((W, W, 3), (226, 224, 220), float)
    weave = np.array(noise(W, W, blur=0.6), float)[..., None] / 255
    blot = np.array(noise(W, W, blur=30), float)[..., None] / 255
    arr = base * (0.94 + 0.06 * weave) * (0.9 + 0.12 * blot)
    # fine weave lines
    arr[::4, :, :] *= 0.985
    arr[:, ::4, :] *= 0.985
    img = Image.fromarray(arr.clip(0, 255).astype(np.uint8))
    # scuffs
    d = ImageDraw.Draw(img, "RGBA")
    for _ in range(40):
        x, y = rng.randint(0, W), rng.randint(0, W)
        r = rng.randint(4, 20)
        d.ellipse([x - r, y - r // 2, x + r, y + r // 2], fill=(150, 140, 135, 8))
    img.save(OUT / "lona.jpg", quality=88)


def turnbuckle():
    S = 128
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    c = S // 2
    # post (steel)
    shadow = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    ImageDraw.Draw(shadow).ellipse([c - 40, c - 34, c + 48, c + 50], fill=(0, 0, 0, 150))
    img = Image.alpha_composite(img, shadow.filter(ImageFilter.GaussianBlur(6)))
    d = ImageDraw.Draw(img)
    d.ellipse([c - 42, c - 42, c + 42, c + 42], fill=(70, 70, 76), outline=(20, 20, 22), width=3)
    d.ellipse([c - 36, c - 36, c + 36, c + 36], fill=(150, 150, 158))
    # pad (dark, padded cushion seen from above)
    d.ellipse([c - 30, c - 30, c + 30, c + 30], fill=(22, 22, 26), outline=(8, 8, 10), width=2)
    d.ellipse([c - 22, c - 26, c + 14, c - 2], fill=(70, 70, 80))
    hl = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    ImageDraw.Draw(hl).ellipse([c - 20, c - 24, c + 4, c - 8], fill=(255, 255, 255, 70))
    img = Image.alpha_composite(img, hl.filter(ImageFilter.GaussianBlur(4)))
    d = ImageDraw.Draw(img)
    # ring of red at the pad's rim
    d.ellipse([c - 30, c - 30, c + 30, c + 30], outline=(180, 20, 24), width=4)
    img.save(OUT / "esquinero.png", optimize=True)


# ---------------------------------------------------------------- icons
def icons():
    S = 128
    white = (245, 245, 245, 255)

    def new():
        img = Image.new("RGBA", (S * 2, S * 2), (0, 0, 0, 0))
        return img, ImageDraw.Draw(img)

    def save(img, name):
        img.resize((S, S), Image.LANCZOS).save(OUT / "iconos" / f"{name}.png", optimize=True)

    k = 2  # draw at 2x then shrink

    img, d = new()
    d.polygon([(70 * k, 44 * k), (70 * k, 212 * k / 2 * 1), (0, 0)], fill=(0, 0, 0, 0))
    d.polygon([(40 * k, 24 * k), (108 * k, 64 * k), (40 * k, 104 * k)], fill=white)
    save(img, "jugar")

    img, d = new()  # ring from above
    d.rectangle([22 * k, 40 * k, 106 * k, 100 * k], outline=white, width=7 * k)
    for i, y in enumerate((52, 64, 76, 88)):
        pass
    d.rectangle([32 * k, 50 * k, 96 * k, 90 * k], outline=white, width=3 * k)
    for x, y in [(22, 40), (106, 40), (22, 100), (106, 100)]:
        d.ellipse([(x - 9) * k, (y - 9) * k, (x + 9) * k, (y + 9) * k], fill=white)
    d.polygon([(22 * k, 100 * k), (106 * k, 100 * k), (114 * k, 116 * k), (14 * k, 116 * k)], fill=white)
    save(img, "ring")

    img, d = new()  # globe
    c, r = 64 * k, 48 * k
    w = 6 * k
    d.ellipse([c - r, c - r, c + r, c + r], outline=white, width=w)
    d.ellipse([c - r // 2, c - r, c + r // 2, c + r], outline=white, width=w)
    d.line([(c, c - r), (c, c + r)], fill=white, width=w)
    d.line([(c - r, c), (c + r, c)], fill=white, width=w)
    for dy in (-26 * k, 26 * k):
        hw = int(math.sqrt(r * r - dy * dy))
        d.line([(c - hw, c + dy), (c + hw, c + dy)], fill=white, width=w - 2)
    save(img, "globo")

    def person(d, cx, top, scale, fill=white):
        hr = 18 * scale
        d.ellipse([cx - hr, top, cx + hr, top + 2 * hr], fill=fill)
        d.pieslice([cx - 36 * scale, top + 2 * hr + 6 * scale, cx + 36 * scale, top + 2 * hr + 78 * scale], 180, 360,
                   fill=fill)

    img, d = new()
    person(d, 64 * k, 24 * k, 1.25 * k)
    save(img, "usuario")

    img, d = new()
    person(d, 40 * k, 30 * k, 0.9 * k, (200, 200, 200, 255))
    person(d, 88 * k, 30 * k, 0.9 * k, (200, 200, 200, 255))
    person(d, 64 * k, 22 * k, 1.1 * k)
    save(img, "grupo")

    img, d = new()  # server stack
    for i in range(3):
        y = (24 + i * 30) * k
        d.rounded_rectangle([26 * k, y, 102 * k, y + 24 * k], 4 * k, outline=white, width=5 * k)
        d.ellipse([84 * k, y + 8 * k, 92 * k, y + 16 * k], fill=white)
    save(img, "servidor")

    img, d = new()  # house
    d.polygon([(64 * k, 18 * k), (114 * k, 62 * k), (14 * k, 62 * k)], fill=white)
    d.rectangle([28 * k, 60 * k, 100 * k, 110 * k], fill=white)
    d.rectangle([54 * k, 78 * k, 74 * k, 110 * k], fill=(0, 0, 0, 0))
    save(img, "casa")

    img, d = new()  # link (two chain rings)
    for (x, y) in [(46, 74), (82, 54)]:
        d.rounded_rectangle([(x - 30) * k, (y - 14) * k, (x + 30) * k, (y + 14) * k], 14 * k, outline=white,
                            width=7 * k)
    img = img.rotate(30, resample=Image.BICUBIC, center=(S, S))
    save(img, "enlace")

    img, d = new()  # exit: door + arrow
    d.rectangle([22 * k, 18 * k, 70 * k, 110 * k], outline=white, width=7 * k)
    d.line([(52 * k, 64 * k), (108 * k, 64 * k)], fill=white, width=9 * k)
    d.polygon([(112 * k, 64 * k), (90 * k, 44 * k), (90 * k, 84 * k)], fill=white)
    save(img, "salir")

    img, d = new()  # picture
    d.rounded_rectangle([16 * k, 26 * k, 112 * k, 102 * k], 6 * k, outline=white, width=6 * k)
    d.polygon([(26 * k, 92 * k), (54 * k, 56 * k), (74 * k, 80 * k), (86 * k, 66 * k), (104 * k, 92 * k)],
              fill=white)
    d.ellipse([78 * k, 36 * k, 94 * k, 52 * k], fill=white)
    save(img, "imagen")

    img, d = new()  # folder
    d.polygon([(14 * k, 30 * k), (52 * k, 30 * k), (60 * k, 40 * k), (114 * k, 40 * k), (114 * k, 104 * k),
               (14 * k, 104 * k)], fill=white)
    save(img, "carpeta")


arena()
button(False)
button(True)
bar()
frame("marco-oro.png", (255, 226, 140), (120, 80, 20), (255, 190, 60))
frame("marco-azul.png", (190, 220, 255), (40, 70, 130), (60, 150, 255))
logo()
canvas()
turnbuckle()
icons()
print("ok")
