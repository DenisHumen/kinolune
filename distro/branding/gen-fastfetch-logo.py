#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Render the Kinolune mark as a terminal logo for fastfetch (DIST-003).

The geometry is the one of design/brand/src/mark.svg (grid 1024):
  moon = circle (512, 512) r=320 minus circle (657.99, 424.01) r=264 (the arc centre
         of the path "A264 264 0 1 1");
  play = triangle (598,322) (598,538) (782,430) with a round 52-unit stroke.
Every text cell is two "pixels" stacked vertically (half blocks), each pixel is
supersampled. $1 is the moon colour, $2 the play colour (set in config.jsonc).

  python3 distro/branding/gen-fastfetch-logo.py > distro/rootfs/usr/share/kinolune-os/fastfetch-logo.txt
"""
import math
import sys

COLS = int(sys.argv[1]) if len(sys.argv) > 1 else 32
X0, X1, Y0, Y1 = 176.0, 848.0, 176.0, 848.0  # crop of the 1024 grid around the mark
ROWS = round(COLS * (Y1 - Y0) / (X1 - X0) / 2)  # terminal cells are ~1:2
SS = 4  # samples per pixel side

MOON_C, MOON_R = (512.0, 512.0), 320.0
CUT_C, CUT_R = (657.9856, 424.0078), 264.0
TRI = [(598.0, 322.0), (598.0, 538.0), (782.0, 430.0)]
STROKE = 26.0  # half of stroke-width 52

FULL, UPPER, LOWER = "█", "▀", "▄"


def in_moon(x, y):
    return math.dist((x, y), MOON_C) <= MOON_R and math.dist((x, y), CUT_C) > CUT_R


def seg_dist(p, a, b):
    dx, dy = b[0] - a[0], b[1] - a[1]
    t = ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / (dx * dx + dy * dy)
    t = max(0.0, min(1.0, t))
    return math.dist(p, (a[0] + t * dx, a[1] + t * dy))


def in_play(x, y):
    sides = [
        (TRI[(i + 1) % 3][0] - TRI[i][0]) * (y - TRI[i][1])
        - (TRI[(i + 1) % 3][1] - TRI[i][1]) * (x - TRI[i][0])
        for i in range(3)
    ]
    if all(v >= 0 for v in sides) or all(v <= 0 for v in sides):
        return True
    return any(seg_dist((x, y), TRI[i], TRI[(i + 1) % 3]) <= STROKE for i in range(3))


def pixel(col, prow):
    """0 = empty, 1 = moon, 2 = play for one half-cell pixel, by coverage."""
    w = (X1 - X0) / COLS
    h = (Y1 - Y0) / (ROWS * 2)
    moon = play = 0
    for i in range(SS):
        for j in range(SS):
            x = X0 + (col + (i + 0.5) / SS) * w
            y = Y0 + (prow + (j + 0.5) / SS) * h
            if in_play(x, y):
                play += 1
            elif in_moon(x, y):
                moon += 1
    half = SS * SS / 2
    if play + moon < half:
        return 0
    return 2 if play >= moon else 1


def cell(top, bot):
    """Glyph and colour of a text cell; where moon meets play the play shape wins."""
    if top and bot:
        if top == bot:
            return FULL, top
        return (UPPER, 2) if top == 2 else (LOWER, 2)
    if top:
        return UPPER, top
    return LOWER, bot


def main():
    lines = []
    for row in range(ROWS):
        out, cur = [], None
        for col in range(COLS):
            top, bot = pixel(col, row * 2), pixel(col, row * 2 + 1)
            if not top and not bot:
                out.append(" ")
                continue
            glyph, color = cell(top, bot)
            if color != cur:
                out.append(f"${color}")
                cur = color
            out.append(glyph)
        lines.append("".join(out).rstrip())

    def blank(s):
        return not s.replace("$1", "").replace("$2", "").strip()

    while lines and blank(lines[0]):
        lines.pop(0)
    while lines and blank(lines[-1]):
        lines.pop()
    sys.stdout.write("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
