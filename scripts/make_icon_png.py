#!/usr/bin/env python3
"""Generate assets/linux/supermd-{128,512}.png from the same SDF
renderer as the macOS icon (scripts/make_icon.py)."""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from make_icon import write_png  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent


def main():
    out_dir = ROOT / "assets" / "linux"
    out_dir.mkdir(parents=True, exist_ok=True)
    for size in (128, 512):
        out = out_dir / f"supermd-{size}.png"
        write_png(out, size)
        print(f"wrote {out}")


if __name__ == "__main__":
    main()
