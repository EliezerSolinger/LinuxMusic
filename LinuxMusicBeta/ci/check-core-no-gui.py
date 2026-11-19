#!/usr/bin/env python3
"""The core never depends on a GUI toolkit (arquitetura-nova.md; interfaces.md, I-15).

For every workspace crate named lm-* that is not lm-gui or lm-gui-*, runs `cargo tree` over every target platform,
every feature and every kind of dependency (normal, build, dev), and fails if any crate in the tree is in
ci/gui-crates.txt. For each hit it prints the path by which the GUI crate came in (`cargo tree -i`).
"""

from __future__ import annotations

import fnmatch
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import workspace  # noqa: E402

TREE = ["--locked", "--target", "all", "--all-features", "--edges", "normal,build,dev"]


def cargo(*args: str) -> str:
    result = subprocess.run(
        ["cargo", *args, "--manifest-path", str(workspace.WORKSPACE / "Cargo.toml")],
        capture_output=True, text=True,
    )
    if result.returncode != 0:
        sys.stderr.write(result.stderr)
        raise SystemExit(f"cargo {' '.join(args)} failed")
    return result.stdout


def main() -> int:
    patterns = workspace.read_list(workspace.CI / "gui-crates.txt")

    def is_gui_crate(name: str) -> bool:
        return any(fnmatch.fnmatchcase(name, pattern) for pattern in patterns)

    failed = False
    checked = 0
    for crate in workspace.members():
        if not workspace.is_core(crate.name):
            print(f"  {crate.name}: not a core crate, skipped")
            continue
        checked += 1
        tree = cargo("tree", "-p", crate.name, *TREE, "--prefix", "none", "--format", "{p}")
        names = {line.split()[0] for line in tree.splitlines() if line.strip()}
        hits = sorted(n for n in names if is_gui_crate(n) and not workspace.is_core(n))
        if not hits:
            print(f"  {crate.name}: ok ({len(names)} crates in the tree)")
            continue
        failed = True
        print(f"  {crate.name}: FAILED, depends on {', '.join(hits)}", file=sys.stderr)
        for hit in hits:
            print(f"\n  how {hit} gets into {crate.name}:", file=sys.stderr)
            sys.stderr.write(cargo("tree", "-p", crate.name, *TREE, "--invert", hit))

    if failed:
        print("\nA core crate (lm-*, not lm-gui-*) depends on a GUI crate. Move the GUI code into an lm-gui-* crate.",
              file=sys.stderr)
        return 1
    print(f"core without GUI: ok ({checked} core crates)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
