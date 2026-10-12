"""MailCraft's app icon artwork, drawn here: a carrier pigeon with a sealed letter in its beak,
in the manner of a wood engraving. Three colours only (ink, paper, the app's colour).

    python draw_art.py [out.png]

writes a 2048 px square PNG (default: mailcraft-art.png beside this file). `cargo xtask icon`
fits it to the tile and makes every size from it. Needs Pillow and NumPy.

Everything is original: the shapes are the curves written below, nothing is traced or copied.
"""

import math
import os
import sys

import numpy as np
from PIL import Image, ImageChops, ImageDraw

OUT = 2048
S = OUT * 2  # drawn at twice the size, then reduced
U = S / 1000.0  # the drawing is written in a 1000-unit square, y down

INK = (0x0B, 0x0B, 0x0C)
PAPER = (0xEF, 0xE9, 0xDC)
FIELD = (0xB3, 0x20, 0x2A)


def cubic(p0, p1, p2, p3, n=64):
    t = np.linspace(0.0, 1.0, n)[:, None]
    p0, p1, p2, p3 = (np.array(p, float) for p in (p0, p1, p2, p3))
    return (1 - t) ** 3 * p0 + 3 * (1 - t) ** 2 * t * p1 + 3 * (1 - t) * t**2 * p2 + t**3 * p3


def curve(start, *segments):
    """A polyline from a start and segments: (point,) is a line, (c1, c2, point) a cubic."""
    points = [np.array(start, float)[None, :]]
    at = np.array(start, float)
    for segment in segments:
        if len(segment) == 1:
            points.append(np.array(segment[0], float)[None, :])
        else:
            c1, c2, to = segment
            points.append(cubic(at, c1, c2, to)[1:])
        at = np.array(segment[-1], float)
    return np.vstack(points)


def px(points):
    return [(float(x) * U, float(y) * U) for x, y in points]


def mask_of(*polygons):
    mask = Image.new("L", (S, S), 0)
    pen = ImageDraw.Draw(mask)
    for polygon in polygons:
        pen.polygon(px(polygon), fill=255)
    return mask


def stroke(pen, points, width, fill=255):
    """A line with round ends and joints."""
    line = px(points)
    w = max(1, int(round(width * U)))
    pen.line(line, fill=fill, width=w, joint="curve")
    r = w / 2.0
    for x, y in (line[0], line[-1]):
        pen.ellipse((x - r, y - r, x + r, y + r), fill=fill)


def arc(centre, radius, a0, a1, n=96):
    a = np.radians(np.linspace(a0, a1, n))
    return np.stack([centre[0] + radius * np.cos(a), centre[1] + radius * np.sin(a)], axis=1)


def rotated(points, centre, degrees):
    a = math.radians(degrees)
    c, s = math.cos(a), math.sin(a)
    p = np.array(points, float) - np.array(centre, float)
    return np.stack([p[:, 0] * c - p[:, 1] * s, p[:, 0] * s + p[:, 1] * c], axis=1) + np.array(centre, float)


def within(layer, mask):
    return ImageChops.multiply(layer, mask)


def layer():
    image = Image.new("L", (S, S), 0)
    return image, ImageDraw.Draw(image)


# ---- the bird ------------------------------------------------------------------------------

# Head, neck and body in one outline: the beak's root, over the crown, down the nape and the
# back, off the right and bottom edges, up the breast and the throat.
BODY = curve(
    (316, 262),
    ((326, 196), (388, 148), (458, 148)),
    ((540, 148), (598, 214), (598, 300)),
    ((598, 376), (640, 452), (706, 512)),
    ((800, 552), (930, 590), (1060, 640)),
    ((1060, 1060),),
    ((236, 1060),),
    ((214, 960), (196, 850), (204, 742)),
    ((212, 640), (286, 524), (332, 444)),
    ((346, 414), (336, 388), (322, 372)),
)

# The wing over the flank: its leading edge comes down across the breast.
WING = curve(
    (452, 640),
    ((520, 540), (668, 508), (792, 560)),
    ((900, 606), (990, 680), (1060, 760)),
    ((1060, 1060),),
    ((690, 1060),),
    ((552, 950), (452, 812), (452, 640)),
)
WING_EDGE = curve(
    (700, 1060),
    ((552, 950), (452, 812), (452, 640)),
    ((520, 540), (668, 508), (792, 560)),
    ((900, 606), (990, 680), (1060, 760)),
)

BEAK_UPPER = curve(
    (324, 284),
    ((272, 284), (212, 308), (170, 348)),
    ((164, 356), (168, 362), (178, 360)),
    ((232, 344), (282, 338), (330, 338)),
)
BEAK_LOWER = curve(
    (330, 338),
    ((282, 340), (236, 348), (190, 362)),
    ((228, 376), (282, 380), (326, 370)),
)
CERE = (300, 280, 36, 21, -14)  # centre x, y, radii, tilt
EYE = (418, 262)

# The letter: held by its upper edge, tilted, running off the left edge.
LETTER_CENTRE = (158, 452)
LETTER = rotated([(-150, -92), (150, -92), (150, 92), (-150, 92)] + np.array(LETTER_CENTRE), LETTER_CENTRE, -13)


def ellipse_points(cx, cy, rx, ry, tilt, n=72):
    a = np.linspace(0, 2 * math.pi, n)
    return rotated(np.stack([cx + rx * np.cos(a), cy + ry * np.sin(a)], axis=1), (cx, cy), tilt)


def draw():
    image = Image.new("RGB", (S, S), FIELD)
    body = mask_of(BODY)
    wing = within(mask_of(WING), body)
    image.paste(PAPER, mask=body)

    # ---- tone, in lines that go round the forms ----
    tone, pen = layer()

    # The back of the head and the nape: rings round the skull, on the side away from the light.
    # The farther out a ring is, the later it begins and the sooner it ends: a crescent, with no
    # edge to it but the ends of its lines.
    for k, radius in enumerate(range(66, 180, 13)):
        stroke(pen, arc((452, 296), radius, -62 + 4.5 * k, 60 - 2.5 * k), 4.6)
    shade = tone

    # The neck: bands that sag round it, close together where the collar shines. They begin
    # farther back on the neck the higher they are, a long one and a short one in turn.
    tone, pen = layer()
    for i, y in enumerate(range(404, 640, 15)):
        sag = 26 + 0.05 * (y - 404)
        line = curve((300, y - 6), ((400, y + sag), (540, y + sag), (720, y - 30)))
        begin = 0.42 - 0.013 * i + (0.07 if i % 2 else 0.0)
        stroke(pen, line[int(begin * len(line)) :], 5.4 if 430 <= y <= 540 else 4.2)
    shade = ImageChops.lighter(shade, within(tone, mask_of([(300, 392), (1060, 392), (1060, 660), (300, 660)])))

    # The collar: a second set across the first, the glint of the neck feathers.
    tone, pen = layer()
    for x in range(380, 760, 15):
        stroke(pen, curve((x, 418), ((x + 24, 462), (x + 30, 506), (x + 18, 552))), 4.0)
    collar = mask_of(curve((470, 428), ((540, 452), (610, 452), (668, 430)), ((690, 470), (716, 506), (744, 532)), ((650, 566), (560, 566), (486, 540)), ((470, 500), (466, 462), (470, 428))))
    shade = ImageChops.lighter(shade, within(tone, collar))

    # The breast: a few lines along its swell, low on the left.
    tone, pen = layer()
    for k, radius in enumerate(range(262, 342, 17)):
        stroke(pen, arc((520, 800), radius, 150 + 3 * k, 214 - 2 * k), 4.2)
    shade = ImageChops.lighter(shade, within(tone, mask_of([(150, 640), (360, 640), (330, 1060), (150, 1060)])))

    # Under the throat, where the head throws its shadow.
    tone, pen = layer()
    for k in range(7):
        stroke(pen, curve((330 + 6 * k, 392 + 9 * k), ((372, 404 + 9 * k), (410, 404 + 9 * k), (452, 388 + 9 * k))), 4.0)
    shade = ImageChops.lighter(shade, within(tone, mask_of([(318, 372), (470, 372), (470, 470), (318, 470)])))

    shade = within(shade, ImageChops.subtract(body, wing))
    image.paste(INK, mask=shade)

    # ---- the wing: rows of feathers, small at the shoulder and long towards the tail ----
    feathers, pen = layer()
    row = 0
    v = 560.0
    while v < 1500:
        size = 34 + 0.085 * (v - 560)  # the half width of a feather in this row
        step = size * 2.0
        u = 200.0 + (step / 2 if row % 2 else 0)
        while u < 1500:
            # A feather: a tongue hanging from (u, v), drawn with the grain turned towards the tail.
            tip = curve((u - size, v), ((u - size, v + size * 1.5), (u + size, v + size * 1.5), (u + size, v)))
            stroke(pen, rotated(tip, (760, 700), -34), 5.6)
            # Its shaded edge: three short lines inside the tongue.
            for k in (1, 2, 3):
                inset = size * (0.18 + 0.2 * k)
                vein = curve((u + size - inset, v + size * 0.2), ((u + size - inset, v + size * 0.9), (u + size * 0.4 - inset * 0.4, v + size * 1.18), (u, v + size * 1.14 - inset * 0.3)))
                stroke(pen, rotated(vein, (760, 700), -34), 3.4)
            u += step
        v += size * 1.12
        row += 1
    image.paste(INK, mask=within(feathers, wing))

    # The dark under the wing's leading edge, on the body in front of it.
    tone, pen = layer()
    for k in range(9):
        stroke(pen, curve((452 - 9 * k, 650 + 6 * k), ((456 - 9 * k, 800), (530 - 9 * k, 940), (676 - 9 * k, 1060))), 4.0)
    cast = within(tone, ImageChops.subtract(body, wing))
    cast = within(cast, mask_of([(360, 640), (700, 640), (700, 1060), (360, 1060)]))
    image.paste(INK, mask=cast)

    # ---- the letter (behind the beak that holds it) ----
    letter = mask_of(LETTER)
    image.paste(PAPER, mask=letter)
    lines, pen = layer()
    corners = [np.array(p) for p in LETTER]
    stroke(pen, np.vstack([corners, corners[:1]]), 9.5)
    point = rotated([np.array(LETTER_CENTRE) + np.array((0, 14))], LETTER_CENTRE, -13)[0]
    stroke(pen, np.vstack([corners[0], point, corners[1]]), 6.5)
    # Paper has a tooth: a few lines along the lower edge.
    for k in (1, 2, 3):
        a = corners[3] + (corners[0] - corners[3]) * (0.09 * k) + (corners[2] - corners[3]) * 0.08
        b = corners[2] + (corners[1] - corners[2]) * (0.09 * k) + (corners[3] - corners[2]) * 0.08
        stroke(pen, np.vstack([a, b]), 3.2)
    image.paste(INK, mask=lines)
    # The seal on the flap's point.
    seal, pen = layer()
    pen.polygon(px(ellipse_points(point[0], point[1], 30, 30, 0)), fill=255)
    image.paste(INK, mask=seal)
    mark, pen = layer()
    stroke(pen, arc(point, 15, 200, 340), 4.4)
    image.paste(PAPER, mask=mark)

    # ---- outlines ----
    outline, pen = layer()
    stroke(pen, BODY, 11.5)
    stroke(pen, WING_EDGE, 9.5)
    image.paste(INK, mask=within(outline, mask_of([(-50, -50), (1050, -50), (1050, 1050), (-50, 1050)])))

    # ---- the beak, the cere and the eye ----
    beak = mask_of(BEAK_UPPER, BEAK_LOWER)
    image.paste(INK, mask=beak)
    gape, pen = layer()
    stroke(pen, curve((186, 357), ((236, 345), (284, 340), (328, 339))), 3.4)
    image.paste(PAPER, mask=gape)
    cere = ellipse_points(*CERE)
    image.paste(PAPER, mask=mask_of(cere))
    rim, pen = layer()
    stroke(pen, cere, 7.5)
    stroke(pen, curve((286, 276), ((296, 268), (310, 268), (320, 276))), 3.2)
    image.paste(INK, mask=rim)

    ex, ey = EYE
    eye, pen = layer()
    pen.polygon(px(ellipse_points(ex, ey, 36, 34, 0)), fill=255)
    image.paste(INK, mask=eye)
    iris, pen = layer()
    pen.polygon(px(ellipse_points(ex, ey, 25, 24, 0)), fill=255)
    image.paste(PAPER, mask=iris)
    pupil, pen = layer()
    pen.polygon(px(ellipse_points(ex, ey, 15, 15, 0)), fill=255)
    image.paste(INK, mask=pupil)
    glint, pen = layer()
    pen.polygon(px(ellipse_points(ex - 6, ey - 6, 5, 5, 0)), fill=255)
    image.paste(PAPER, mask=glint)
    # The lid, and the small feathers that run back from the eye.
    lid, pen = layer()
    stroke(pen, arc((ex, ey), 47, 205, 318), 4.4)
    for k in range(4):
        stroke(pen, curve((ex + 44 + 12 * k, ey + 20 + 9 * k), ((ex + 70 + 12 * k, ey + 26 + 9 * k), (ex + 92 + 10 * k, ey + 44 + 9 * k), (ex + 104 + 8 * k, ey + 66 + 9 * k))), 3.6)
    image.paste(INK, mask=within(lid, body))

    return image.resize((OUT, OUT), Image.LANCZOS)


if __name__ == "__main__":
    here = os.path.dirname(os.path.abspath(__file__))
    out = sys.argv[1] if len(sys.argv) > 1 else os.path.join(here, "mailcraft-art.png")
    draw().save(out)
    print(out)
