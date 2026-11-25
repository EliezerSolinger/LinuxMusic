#!/usr/bin/env bash
# ci/run.sh <command> [args...]
#
# Runs the command with its output as usual. When it fails on GitHub Actions, the end of the output also becomes an
# error annotation on the run, so the reason can be read without the full log (the annotations are public through the
# API; the logs need a login).
set -uo pipefail

log="$(mktemp)"
"$@" 2>&1 | tee "$log"
status="${PIPESTATUS[0]}"
if [ "$status" -ne 0 ] && [ -n "${GITHUB_ACTIONS:-}" ]; then
	python3 - "$log" "$*" "$status" <<'PY'
import re, sys
path, command, status = sys.argv[1:]
text = open(path, encoding="utf-8", errors="replace").read()
text = re.sub(r"\x1b\[[0-9;]*[A-Za-z]", "", text)
lines = [line for line in text.splitlines() if line.strip()]
# GitHub keeps about 4 KB of each annotation and 10 annotations per step, so the reasons go in several: (1) the
# failed targets and the sanitizers' summaries, with the last lines if they fit; (2) the panics with their message
# (since Rust 1.92 the line has the thread's id: "thread 'name' (1234) panicked at"); (3) each distinct leak of
# LeakSanitizer with the test target it came from and its first frames of our own code (or of the C library)
LIMIT = 3900
notes = []
errors = [l.strip() for l in lines if re.match(r"\s*(error|FAILED|=+\d*=+ERROR|SUMMARY:)", l)]
main = "\n".join(errors)[:LIMIT - 1200]
notes.append((main + "\n...\n(last lines)\n" if main else "") + "\n".join(lines[-40:])[-(LIMIT - len(main) - 30):])
panics = []
for i, l in enumerate(lines):
    if re.match(r"\s*thread '.*'( \(\d+\))? panicked", l):
        panics.append("\n".join(x.strip() for x in lines[i:i + 3])[:600])
def chunks(items, count):
    out, cur = [], ""
    for it in items:
        if len(cur) + len(it) + 1 > LIMIT:
            out.append(cur)
            cur = ""
            if len(out) == count:
                return out
        cur += it + "\n"
    if cur:
        out.append(cur)
    return out[:count]
notes += chunks(panics, 2)
skip = r"(asan|lsan|sanitizer_common|/alloc/src/|/alloc/|__rust_|alloc::alloc|<alloc::|core::ptr|libc_start|_start\b|std::rt::|std::panicking|test::run_test|__libc|std::sys::|core::ops::function)"
leaks, seen = [], set()
for i, l in enumerate(lines):
    if not re.match(r"\s*(Direct|Indirect) leak of", l):
        continue
    target = next((x for x in lines[i:] if "error: test failed" in x), "")
    target = re.sub(r".*rerun pass `(.*)`.*", r"\1", target)
    frames = []
    for f in lines[i + 1:i + 80]:
        if not re.match(r"\s*#\d+ ", f):
            break
        if not re.search(skip, f):
            f = re.sub(r"^\s*#\d+ 0x[0-9a-f]+ in ", "", f.strip())
            f = re.sub(r"/home/runner/\S*?/(crates|library)/", r"\1/", f)
            frames.append(f[:200])
    key = (target, tuple(frames[:6]))
    if key in seen:
        continue
    seen.add(key)
    leaks.append(f"[{target}] {l.strip()}\n  " + "\n  ".join(frames[:8]))
notes += chunks(leaks, 10 - len(notes))
def esc(s, prop=False):
    s = s.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")
    return s.replace(":", "%3A").replace(",", "%2C") if prop else s
for n, note in enumerate(notes):
    part = f" ({n + 1}/{len(notes)})" if len(notes) > 1 else ""
    print(f"::error title={esc(command[:100], True)} (exit {status}){part}::{esc(note)}")
PY
fi
rm -f "$log"
exit "$status"
