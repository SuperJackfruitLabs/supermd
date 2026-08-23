#!/usr/bin/env python3
"""Generate assets/icon.icns: supermd's app icon (accent-red rounded
square, white block M), pure Python + macOS sips/iconutil. Rerun to
regenerate; the committed .icns is the build input."""

import math
import struct
import subprocess
import tempfile
import zlib
from pathlib import Path

SIZE = 1024
ROOT = Path(__file__).resolve().parent.parent


def sd_rounded_rect(x, y, cx, cy, half, radius):
    qx = abs(x - cx) - (half - radius)
    qy = abs(y - cy) - (half - radius)
    ax, ay = max(qx, 0.0), max(qy, 0.0)
    return math.hypot(ax, ay) + min(max(qx, qy), 0.0) - radius


def sd_segment(x, y, ax, ay, bx, by):
    px, py = x - ax, y - ay
    dx, dy = bx - ax, by - ay
    h = max(0.0, min(1.0, (px * dx + py * dy) / (dx * dx + dy * dy)))
    return math.hypot(px - dx * h, py - dy * h)


SEGS = [
    (310, 700, 310, 330),
    (310, 330, 512, 565),
    (512, 565, 714, 330),
    (714, 330, 714, 700),
]
STROKE = 48.0


def pixel(x, y):
    d_rect = sd_rounded_rect(x, y, 512, 512, 460, 200)
    if d_rect > 1.0:
        return (0, 0, 0, 0)
    edge = max(0.0, min(1.0, 0.5 - d_rect))
    t = y / SIZE
    bg = (
        int(0xE2 + (0xC9 - 0xE2) * t),
        int(0x5D + (0x3A - 0x5D) * t),
        int(0x5F + (0x3E - 0x5F) * t),
    )
    d_m = min(sd_segment(x, y, *s) for s in SEGS) - STROKE
    if d_m < 1.0:
        w = max(0.0, min(1.0, 0.5 - d_m))
        r = int(bg[0] + (255 - bg[0]) * w)
        g = int(bg[1] + (255 - bg[1]) * w)
        b = int(bg[2] + (255 - bg[2]) * w)
        return (r, g, b, int(255 * edge))
    return (bg[0], bg[1], bg[2], int(255 * edge))


def write_png(path, size):
    rows = b""
    for y in range(size):
        row = b"\x00"
        for x in range(size):
            sx = (x + 0.5) * SIZE / size
            sy = (y + 0.5) * SIZE / size
            row += bytes(pixel(sx, sy))
        rows += row

    def chunk(tag, data):
        c = struct.pack(">I", len(data)) + tag + data
        return c + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(rows))
    png += chunk(b"IEND", b"")
    Path(path).write_bytes(png)


def main():
    with tempfile.TemporaryDirectory() as tmp:
        iconset = Path(tmp) / "supermd.iconset"
        iconset.mkdir()
        base = iconset / "icon_512x512@2x.png"
        print("rendering 1024px master…")
        write_png(base, 1024)
        for name, px in [
            ("icon_512x512.png", 512),
            ("icon_256x256@2x.png", 512),
            ("icon_256x256.png", 256),
            ("icon_128x128@2x.png", 256),
            ("icon_128x128.png", 128),
            ("icon_32x32@2x.png", 64),
            ("icon_32x32.png", 32),
            ("icon_16x16@2x.png", 32),
            ("icon_16x16.png", 16),
        ]:
            subprocess.run(
                ["sips", "-z", str(px), str(px), str(base), "--out", str(iconset / name)],
                check=True,
                capture_output=True,
            )
        out = ROOT / "assets" / "icon.icns"
        subprocess.run(["iconutil", "-c", "icns", str(iconset), "-o", str(out)], check=True)
        print(f"wrote {out} ({out.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
