#!/usr/bin/env python3
"""Every dependency of a workspace crate comes from `[workspace.dependencies]` (docs/implementacao.md: "uma
dependência nova do workspace é pedida à sessão de design"; interfaces.md, I-17: the allowed crates).

The list of external crates the program may use is then one list, in the workspace Cargo.toml, where cargo-deny and
the reviewers see it. Each dependency of a crate (normal, dev, build, and their per-target forms) must be written
`name.workspace = true` or `name = { workspace = true, ... }`; the crate may add `features` and `optional`, nothing
that picks another version or source.
"""

from __future__ import annotations

import sys
import tomllib
from pathlib import Path

sys.dont_write_bytecode = True  # no __pycache__ in the source tree
sys.path.insert(0, str(Path(__file__).resolve().parent))
import workspace  # noqa: E402

KINDS = ("dependencies", "dev-dependencies", "build-dependencies")
ALLOWED_KEYS = {"workspace", "features", "optional"}


def dependency_tables(manifest: dict):
    for kind in KINDS:
        if kind in manifest:
            yield kind, manifest[kind]
    for target, table in (manifest.get("target") or {}).items():
        for kind in KINDS:
            if kind in table:
                yield f"target.{target}.{kind}", table[kind]


def main() -> int:
    problems = []
    for crate in workspace.members():
        manifest = tomllib.loads(crate.manifest.read_text(encoding="utf-8"))
        where = crate.manifest.relative_to(workspace.WORKSPACE)
        for table_name, table in dependency_tables(manifest):
            for name, spec in table.items():
                if not (isinstance(spec, dict) and spec.get("workspace") is True):
                    problems.append(f"{where} [{table_name}] {name}: write `{name}.workspace = true` and add {name} "
                                    f"to [workspace.dependencies] in the workspace Cargo.toml")
                    continue
                # (`default-features` is refused too: on an inherited dependency Cargo ignores it)
                extra = set(spec) - ALLOWED_KEYS
                if extra:
                    problems.append(f"{where} [{table_name}] {name}: {', '.join(sorted(extra))} belongs in "
                                    f"[workspace.dependencies], not in the crate")
        print(f"  {crate.name}: {'FAILED' if any(str(where) in p for p in problems) else 'ok'}")

    if problems:
        print("\ndependencies outside [workspace.dependencies]:", file=sys.stderr)
        for problem in problems:
            print(f"  - {problem}", file=sys.stderr)
        return 1
    print("workspace dependencies: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
