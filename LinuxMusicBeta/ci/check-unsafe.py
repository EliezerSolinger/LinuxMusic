#!/usr/bin/env python3
"""`unsafe` lives only in the lm-sys-* crates (interfaces.md, Parte 2, rule 9).

For every workspace crate that is not lm-sys-*, this fails if:
  - its Cargo.toml does not opt into the workspace lints (`[lints] workspace = true`), which is where
    `unsafe_code = "forbid"` comes from;
  - the workspace no longer forbids `unsafe_code`;
  - any of its .rs files (sources, tests, examples, benches, build script) has the `unsafe` keyword or tries to
    lift the lint (`allow(unsafe_code)`, `expect(unsafe_code)`, `warn(unsafe_code)`), outside comments and strings.

The compiler already refuses `unsafe` under `forbid`; this check also catches a crate that forgot the lints, and
code the compiler would not see (a file behind a cfg that is off on this platform).
"""

from __future__ import annotations

import re
import sys
import tomllib
from pathlib import Path

sys.dont_write_bytecode = True  # no __pycache__ in the source tree
sys.path.insert(0, str(Path(__file__).resolve().parent))
import workspace  # noqa: E402

UNSAFE = re.compile(r"\bunsafe\b")
LIFTED = re.compile(r"\b(allow|expect|warn)\s*\([^)]*\bunsafe_code\b")


def strip_comments_and_strings(source: str) -> str:
    """The code with comments, string, byte-string, raw-string and char literals blanked (newlines kept)"""
    out = []
    i, n = 0, len(source)

    def blank(text: str) -> str:
        return "".join("\n" if ch == "\n" else " " for ch in text)

    while i < n:
        ch = source[i]
        two = source[i:i + 2]
        if two == "//":
            end = source.find("\n", i)
            end = n if end < 0 else end
            out.append(blank(source[i:end]))
            i = end
        elif two == "/*":
            depth, j = 1, i + 2
            while j < n and depth:
                if source.startswith("/*", j):
                    depth, j = depth + 1, j + 2
                elif source.startswith("*/", j):
                    depth, j = depth - 1, j + 2
                else:
                    j += 1
            out.append(blank(source[i:j]))
            i = j
        elif (m := re.match(r'(?:b|c)?r(#*)"', source[i:])) and (i == 0 or not (source[i - 1].isalnum() or source[i - 1] == "_")):
            hashes = m.group(1)
            close = '"' + hashes
            end = source.find(close, i + m.end())
            end = n if end < 0 else end + len(close)
            out.append(blank(source[i:end]))
            i = end
        elif ch == '"' or (two in ('b"', 'c"') and (i == 0 or not (source[i - 1].isalnum() or source[i - 1] == "_"))):
            j = i + (1 if ch == '"' else 2)
            while j < n and source[j] != '"':
                j += 2 if source[j] == "\\" else 1
            out.append(blank(source[i:j + 1]))
            i = j + 1
        elif ch == "'":
            # A char literal ('a', '\n', '\u{1F600}', b'x' handled as b + this) or a lifetime/label ('a, 'static)
            m = re.match(r"'(?:\\(?:u\{[0-9a-fA-F_]+\}|x[0-9a-fA-F]{2}|.)|[^\\'\n])'", source[i:])
            if m:
                out.append(blank(m.group(0)))
                i += m.end()
            else:
                out.append(ch)
                i += 1
        else:
            out.append(ch)
            i += 1
    return "".join(out)


def main() -> int:
    problems: list[str] = []

    root_manifest = tomllib.loads((workspace.WORKSPACE / "Cargo.toml").read_text(encoding="utf-8"))
    level = root_manifest.get("workspace", {}).get("lints", {}).get("rust", {}).get("unsafe_code")
    if isinstance(level, dict):
        level = level.get("level")
    if level != "forbid":
        problems.append(f"Cargo.toml: [workspace.lints.rust] unsafe_code must be \"forbid\" (it is {level!r})")

    checked = 0
    for crate in workspace.members():
        if workspace.is_sys(crate.name):
            print(f"  {crate.name}: lm-sys-* crate, unsafe allowed")
            continue
        checked += 1
        before = len(problems)
        manifest = tomllib.loads(crate.manifest.read_text(encoding="utf-8"))
        lints = manifest.get("lints")
        if not (isinstance(lints, dict) and lints.get("workspace") is True):
            problems.append(
                f"{crate.manifest.relative_to(workspace.WORKSPACE)}: {crate.name} must opt into the workspace lints "
                f"with\n      [lints]\n      workspace = true\n    (only lm-sys-* crates may use unsafe)"
            )
        for path in crate.rust_files():
            code = strip_comments_and_strings(path.read_text(encoding="utf-8", errors="replace"))
            for number, line in enumerate(code.splitlines(), start=1):
                where = f"{path.relative_to(workspace.WORKSPACE)}:{number}"
                if UNSAFE.search(line):
                    problems.append(f"{where}: `unsafe` in {crate.name}; unsafe code belongs in an lm-sys-* crate")
                if LIFTED.search(line):
                    problems.append(f"{where}: {crate.name} lifts the unsafe_code lint; only lm-sys-* crates may")
        print(f"  {crate.name}: {'ok' if len(problems) == before else 'FAILED'}")

    if problems:
        print("\nunsafe policy violations (interfaces.md, Parte 2, rule 9):", file=sys.stderr)
        for problem in problems:
            print(f"  - {problem}", file=sys.stderr)
        return 1
    print(f"unsafe policy: ok ({checked} crates without unsafe)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
