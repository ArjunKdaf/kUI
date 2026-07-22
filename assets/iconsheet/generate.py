#!/usr/bin/env python3
"""kUI icon sheet generator - the base Arjun styles from.

Produces assets@2x.png: 256x256, transparent, sprites authored in pure
white (the runtime multiplies by the theme color) at the exact rects the
launcher/frontend constants expect (1x coords x2). Only the seven
sprites kUI reads are drawn; the rest of the sheet stays empty.

Drawn at 8x and downscaled for clean edges on the arcs.
"""

from PIL import Image, ImageDraw

SS = 8  # supersample factor on top of the @2x sheet
W = 255  # pure white; alpha carries the shape


def canvas(w, h):
    return Image.new("RGBA", (w * SS, h * SS), (0, 0, 0, 0))


def down(img, w, h):
    return img.resize((w, h), Image.LANCZOS)


def rr(d, box, r, fill=None, outline=None, width=1):
    d.rounded_rectangle(box, radius=r, fill=fill, outline=outline, width=width)


def battery_body():
    # 34x20: outline body, tip nub on the LEFT (the fill drains toward it)
    w, h = 34, 20
    img = canvas(w, h)
    d = ImageDraw.Draw(img)
    s = SS
    # tip: 4px wide, centered 8px tall, at the left edge
    rr(d, (0, 6 * s, 4 * s, 14 * s), 1 * s, fill=(W, W, W, 255))
    # body: from x=3 to right edge, 2px stroke
    rr(d, (3 * s, 0, (w - 1) * s, (h - 1) * s), 4 * s,
       outline=(W, W, W, 255), width=2 * s)
    return down(img, w, h)


def battery_fill():
    # 24x12 solid block; the runtime crops its width to the charge level
    w, h = 24, 12
    img = canvas(w, h)
    d = ImageDraw.Draw(img)
    rr(d, (0, 0, w * SS - 1, h * SS - 1), 2 * SS, fill=(W, W, W, 255))
    return down(img, w, h)


def bolt_polygon(s):
    # lightning bolt in a 32x20 box, scaled by s
    pts = [(19, 1), (10, 11), (15, 11), (13, 19), (22, 9), (17, 9)]
    return [(x * s, y * s) for (x, y) in pts]


def battery_bolt():
    # 32x20: SOLID body (matching battery_body minus the tip) with a
    # bolt-shaped transparent knockout - shown only at full charge
    w, h = 32, 20
    img = canvas(w, h)
    d = ImageDraw.Draw(img)
    rr(d, (1 * SS, 0, (w - 1) * SS, (h - 1) * SS), 4 * SS, fill=(W, W, W, 255))
    # knockout: punch alpha out with the bolt polygon
    hole = Image.new("L", img.size, 0)
    hd = ImageDraw.Draw(hole)
    hd.polygon(bolt_polygon(SS), fill=255)
    alpha = img.getchannel("A")
    alpha.paste(0, mask=hole)
    img.putalpha(alpha)
    return down(img, w, h)


def wifi():
    # 24x24: dot + two arcs, opening upward
    w = h = 24
    img = canvas(w, h)
    d = ImageDraw.Draw(img)
    s = SS
    cx, cy = 12 * s, 20 * s
    d.ellipse((cx - 2.2 * s, cy - 2.2 * s, cx + 2.2 * s, cy + 2.2 * s),
              fill=(W, W, W, 255))
    for r in (8, 14):
        d.arc((cx - r * s, cy - r * s, cx + r * s, cy + r * s),
              start=222, end=318, fill=(W, W, W, 255), width=int(2.6 * s))
    return down(img, w, h)


def brightness():
    # 38x38: sun - core circle + 8 rays
    w = h = 38
    img = canvas(w, h)
    d = ImageDraw.Draw(img)
    s = SS
    cx = cy = 19 * s
    d.ellipse((cx - 7 * s, cy - 7 * s, cx + 7 * s, cy + 7 * s),
              fill=(W, W, W, 255))
    import math
    for i in range(8):
        a = math.pi / 4 * i
        x0 = cx + math.cos(a) * 11 * s
        y0 = cy + math.sin(a) * 11 * s
        x1 = cx + math.cos(a) * 17 * s
        y1 = cy + math.sin(a) * 17 * s
        d.line((x0, y0, x1, y1), fill=(W, W, W, 255), width=int(2.8 * s))
    return down(img, w, h)


def volume():
    # 38x38: speaker wedge + two arcs
    w = h = 38
    img = canvas(w, h)
    d = ImageDraw.Draw(img)
    s = SS
    d.polygon([(4 * s, 14 * s), (10 * s, 14 * s), (18 * s, 6 * s),
               (18 * s, 32 * s), (10 * s, 24 * s), (4 * s, 24 * s)],
              fill=(W, W, W, 255))
    cx, cy = 18 * s, 19 * s
    for r in (7, 12):
        d.arc((cx - r * s, cy - r * s, cx + r * s, cy + r * s),
              start=-55, end=55, fill=(W, W, W, 255), width=int(2.8 * s))
    return down(img, w, h)


def bluetooth():
    # 24x24: the rune - vertical stroke, two chevrons, two cross lines
    w = h = 24
    img = canvas(w, h)
    d = ImageDraw.Draw(img)
    s = SS
    lw = int(2.4 * s)
    d.line((12 * s, 1 * s, 12 * s, 23 * s), fill=(W, W, W, 255), width=lw)
    d.line((12 * s, 1 * s, 19 * s, 7 * s), fill=(W, W, W, 255), width=lw)
    d.line((19 * s, 7 * s, 5 * s, 17 * s), fill=(W, W, W, 255), width=lw)
    d.line((12 * s, 23 * s, 19 * s, 17 * s), fill=(W, W, W, 255), width=lw)
    d.line((19 * s, 17 * s, 5 * s, 7 * s), fill=(W, W, W, 255), width=lw)
    return down(img, w, h)


def main():
    sheet = Image.new("RGBA", (256, 256), (0, 0, 0, 0))
    # (sprite, 1x coords from the launcher/frontend constants) -> x2
    for img, (x, y) in [
        (battery_body(), (47, 51)),
        (battery_fill(), (81, 33)),
        (battery_bolt(), (91, 51)),
        (wifi(), (1, 104)),
        (brightness(), (1, 85)),
        (volume(), (21, 85)),
        (bluetooth(), (53, 104)),
    ]:
        sheet.alpha_composite(img, (x * 2, y * 2))
    out = "assets@2x.png"
    sheet.save(out)
    # preview on dark, sprites tinted-ish, for humans
    prev = Image.new("RGBA", (256, 256), (24, 24, 24, 255))
    prev.alpha_composite(sheet)
    prev.save("preview.png")
    print("wrote", out, "and preview.png")


if __name__ == "__main__":
    main()
