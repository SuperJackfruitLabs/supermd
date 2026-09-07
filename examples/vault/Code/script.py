#!/usr/bin/env python3
"""Generate a vault of linked notes, for measuring index cost.

Not used by the app. It is here so the Python viewer has a real file,
and because a 2000-note vault is the interesting case for the knowledge
index, which walks the whole tree when a workspace opens.
"""

from __future__ import annotations

import random
from pathlib import Path

WORDS = "note link graph markdown editor plugin theme render index".split()


def body(index: int, total: int) -> str:
    lines = [f"# Note {index}", ""]
    lines += [" ".join(random.choice(WORDS) for _ in range(12)) for _ in range(40)]
    a, b = random.randrange(total), random.randrange(total)
    lines.append(f"See [[Note {a}]] and [[Note {b}]].")
    return "\n".join(lines)


def generate(root: Path, total: int = 2000, per_folder: int = 100) -> None:
    for i in range(total):
        folder = root / f"folder{i // per_folder}"
        folder.mkdir(parents=True, exist_ok=True)
        (folder / f"note{i}.md").write_text(body(i, total), encoding="utf-8")
    print(f"wrote {total} notes to {root}")


if __name__ == "__main__":
    generate(Path.home() / "supermd-perf-vault")
