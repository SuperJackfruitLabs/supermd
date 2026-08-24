#!/usr/bin/env python3
"""Generate assets/dmg/bg@2x.png: the DMG window background (1320x840
for a 660x420 window @2x). Jackfruit paper, faint app-icon motif on the
left drop zone, an amber arrow pointing at the Applications side."""

import math
import struct
import zlib
from pathlib import Path

W, H = 1320, 840
ROOT = Path(__file__).resolve().parent.parent

PAPER = (0xFD, 0xFB, 0xF6)
INK = (0x91, 0x8B, 0x7D)
AMBER = (0xC9, 0x82, 0x1C)


def sd_rounded_rect(x, y, cx, cy, hw, hh, r):
    qx = abs(x - cx) - (hw - r)
    qy = abs(y - cy) - (hh - r)
    ax, ay = max(qx, 0.0), max(qy, 0.0)
    return math.hypot(ax, ay) + min(max(qx, qy), 0.0) - r


def sd_segment(x, y, ax, ay, bx, by):
    px, py = x - ax, y - ay
    dx, dy = bx - ax, by - ay
    h = max(0.0, min(1.0, (px * dx + py * dy) / (dx * dx + dy * dy)))
    return math.hypot(px - dx * h, py - dy * h)


# Arrow: shaft + head, centered at (660, 420), pointing right.
ARROW = [
    (560, 420, 730, 420),
    (730, 420, 690, 380),
    (730, 420, 690, 460),
]
ARROW_W = 10.0

# Faint icon outline behind the app-icon position (330, 420).
ICON_CX, ICON_CY, ICON_HALF, ICON_R = 330, 400, 130, 56


def mix(a, b, t):
    return tuple(int(a[i] + (b[i] - a[i]) * t) for i in range(3))


def pixel(x, y):
    c = PAPER
    # faint drop-zone outline
    d = abs(sd_rounded_rect(x, y, ICON_CX, ICON_CY, ICON_HALF, ICON_HALF, ICON_R)) - 2.0
    if d < 1.0:
        c = mix(c, INK, 0.25 * max(0.0, min(1.0, 0.5 - d)))
    d = abs(sd_rounded_rect(x, y, 990, ICON_CY, ICON_HALF, ICON_HALF, ICON_R)) - 2.0
    if d < 1.0:
        c = mix(c, INK, 0.25 * max(0.0, min(1.0, 0.5 - d)))
    # arrow
    da = min(sd_segment(x, y, *s) for s in ARROW) - ARROW_W
    if da < 1.0:
        c = mix(c, AMBER, max(0.0, min(1.0, 0.5 - da)))
    return c


def main():
    rows = b""
    for y in range(H):
        row = b"\x00"
        for x in range(W):
            row += bytes(pixel(x + 0.5, y + 0.5))
        rows += row

    def chunk(tag, data):
        c = struct.pack(">I", len(data)) + tag + data
        return c + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", W, H, 8, 2, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(rows))
    png += chunk(b"IEND", b"")
    out = ROOT / "assets" / "dmg" / "bg@2x.png"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_bytes(png)
    print(f"wrote {out} ({out.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
