#!/usr/bin/env python3
"""Generate assets/windows/supermd.ico (PNG-compressed ICO with
16/32/48/256 entries) using the shared SDF renderer."""

import struct
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from make_icon import write_png  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
SIZES = [16, 32, 48, 256]


def main():
    out_dir = ROOT / "assets" / "windows"
    out_dir.mkdir(parents=True, exist_ok=True)
    blobs = []
    with tempfile.TemporaryDirectory() as tmp:
        for size in SIZES:
            p = Path(tmp) / f"{size}.png"
            write_png(p, size)
            blobs.append(p.read_bytes())

    header = struct.pack("<HHH", 0, 1, len(SIZES))
    entries = b""
    offset = len(header) + 16 * len(SIZES)
    for size, blob in zip(SIZES, blobs):
        dim = 0 if size == 256 else size
        entries += struct.pack(
            "<BBBBHHII", dim, dim, 0, 0, 1, 32, len(blob), offset
        )
        offset += len(blob)
    out = out_dir / "supermd.ico"
    out.write_bytes(header + entries + b"".join(blobs))
    print(f"wrote {out} ({out.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
