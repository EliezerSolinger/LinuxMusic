#!/usr/bin/env python3
"""What the CI needs to know about the workspace, read from `cargo metadata` and the crates' files.

Every CI step is generic over the workspace: it asks this script which crates exist and which of them take part in
each check, so a crate that does not exist yet never breaks the CI and a new crate is picked up without editing the
workflow.

    ci/workspace.py members        every workspace crate, one per line
    ci/workspace.py loom           the crates with `cfg(loom)` code (sources, tests or Cargo.toml)
    ci/workspace.py miri           the crates that run under miri (ci/miri-crates.txt, or opted in by metadata)

A crate opts into miri either by being listed in `ci/miri-crates.txt` or with, in its own Cargo.toml:

    [package.metadata.lm-ci]
    miri = true

Run from anywhere; the workspace is the parent folder of this script's folder.
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

WORKSPACE = Path(__file__).resolve().parent.parent
CI = WORKSPACE / "ci"

# In the code: `cfg(loom)`, `cfg(all(test, loom))`, `cfg(not(loom))`, `cfg!(loom)`...
LOOM_CFG = re.compile(r"\bcfg!?\s*\([^\n]*\bloom\b")
# In Cargo.toml: `[target.'cfg(loom)'.dependencies]` (and not the `check-cfg` that only declares the name)
LOOM_TARGET = re.compile(r"^\s*\[target\.['\"]cfg\([^\n]*\bloom\b", re.MULTILINE)


@dataclass(frozen=True)
class Crate:
    name: str
    manifest: Path
    metadata: dict

    @property
    def root(self) -> Path:
        return self.manifest.parent

    def rust_files(self) -> list[Path]:
        """Every .rs file of the crate (sources, tests, examples, benches, build script), without build outputs"""
        files = []
        for path in sorted(self.root.rglob("*.rs")):
            relative = path.relative_to(self.root).parts
            if "target" in relative or any(part.startswith(".") for part in relative):
                continue
            files.append(path)
        return files


def cargo_metadata() -> dict:
    result = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--no-deps", "--locked",
         "--manifest-path", str(WORKSPACE / "Cargo.toml")],
        check=True, capture_output=True, text=True,
    )
    return json.loads(result.stdout)


def members() -> list[Crate]:
    meta = cargo_metadata()
    ids = set(meta["workspace_members"])
    crates = [
        Crate(p["name"], Path(p["manifest_path"]), p.get("metadata") or {})
        for p in meta["packages"] if p["id"] in ids
    ]
    return sorted(crates, key=lambda c: c.name)


def is_sys(name: str) -> bool:
    """The only crates allowed to contain `unsafe` (interfaces.md, Parte 2, rule 9)"""
    return name.startswith("lm-sys-")


def is_gui(name: str) -> bool:
    """The crates allowed to depend on a GUI toolkit"""
    return name == "lm-gui" or name.startswith("lm-gui-")


def is_core(name: str) -> bool:
    """The core: every lm-* crate that is not a GUI crate (these must never pull a GUI toolkit)"""
    return name.startswith("lm-") and not is_gui(name)


def loom_crates() -> list[Crate]:
    found = []
    for crate in members():
        manifest = crate.manifest.read_text(encoding="utf-8")
        sources = [f.read_text(encoding="utf-8", errors="replace") for f in crate.rust_files()]
        if LOOM_TARGET.search(manifest) or any(LOOM_CFG.search(text) for text in sources):
            found.append(crate)
    return found


def read_list(path: Path) -> list[str]:
    """A list file: one name per line, `#` starts a comment"""
    names = []
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.split("#", 1)[0].strip()
        if line:
            names.append(line)
    return names


def miri_crates() -> list[Crate]:
    listed = set(read_list(CI / "miri-crates.txt"))
    return [
        c for c in members()
        if c.name in listed or (c.metadata.get("lm-ci") or {}).get("miri") is True
    ]


def main(argv: list[str]) -> int:
    commands = {"members": members, "loom": loom_crates, "miri": miri_crates}
    if len(argv) != 2 or argv[1] not in commands:
        print(f"usage: {argv[0]} {{{'|'.join(commands)}}}", file=sys.stderr)
        return 2
    for crate in commands[argv[1]]():
        print(crate.name)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
