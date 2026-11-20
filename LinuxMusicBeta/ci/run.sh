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
# The compiler's and the tests' errors first, then the last lines
# The panics with the line after each (the message), then the compiler's and the tests' errors, then the last lines
panics = []
for i, l in enumerate(lines):
    if re.match(r"\s*thread '.*' panicked", l) and len(panics) < 30:
        panics += lines[i:i + 2]
errors = [l for l in lines if re.match(r"\s*(error|FAILED|  - |=+\d*=+ERROR|SUMMARY:)", l)]
head = panics + errors[:30]
body = "\n".join((head + ["...", "(last lines)"] if head else []) + lines[-40:])
def esc(s, prop=False):
    s = s.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")
    return s.replace(":", "%3A").replace(",", "%2C") if prop else s
print(f"::error title={esc(command[:120], True)} (exit {status})::{esc(body[-12000:])}")
PY
fi
rm -f "$log"
exit "$status"
