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
# The panics with the line after each (the message), then the leaks with their first frames of our own code, then
# the compiler's and the tests' errors, then the last lines. (Since Rust 1.92 the panic line has the thread's id:
# "thread 'name' (1234) panicked at".)
panics = []
for i, l in enumerate(lines):
    if re.match(r"\s*thread '.*'( \(\d+\))? panicked", l) and len(panics) < 30:
        panics += lines[i:i + 2]
# LeakSanitizer: each "Direct leak of" with its frames, minus the allocator's own (up to 12 frames, 12 leaks)
leaks = []
for i, l in enumerate(lines):
    if re.match(r"\s*Direct leak of", l) and len(leaks) < 12 * 13:
        frames = []
        for f in lines[i + 1:i + 60]:
            if not re.match(r"\s*#\d+ ", f):
                break
            if not re.search(r"(asan|lsan|sanitizer_common|/alloc/src/|__rust_|alloc::alloc|<alloc::|core::ptr)", f):
                frames.append(re.sub(r"\s+", " ", f.strip())[:220])
        leaks += [l.strip()] + frames[:12]
errors = [l for l in lines if re.match(r"\s*(error|FAILED|  - |=+\d*=+ERROR|SUMMARY:)", l)]
head = panics + leaks + errors[:30]
head_text = "\n".join(head)[:9000]
body = (head_text + "\n...\n(last lines)\n" if head else "") + "\n".join(lines[-40:])[-3000:]
def esc(s, prop=False):
    s = s.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")
    return s.replace(":", "%3A").replace(",", "%2C") if prop else s
print(f"::error title={esc(command[:120], True)} (exit {status})::{esc(body)}")
PY
fi
rm -f "$log"
exit "$status"
