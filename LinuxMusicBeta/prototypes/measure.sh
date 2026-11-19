#!/bin/sh
# Measures the GPUI prototype: frame times (from the app, see fake-core/src/bench.rs), CPU and memory (from outside).
# Usage: ./measure.sh [runs]   (from prototypes/, after a release build into ../target)
# Each mode runs LM_BENCH_SECS=20 measured seconds after 2 s of warm-up. CPU % = process CPU time over wall time
# between second 3 and second 21 (100 % = one core). Memory = RSS and the macOS phys_footprint at second 21.
# Keep the window in front and the machine otherwise quiet: an occluded window gets no display-link frames.
set -eu
BIN=${BIN:-../target/release/proto-gpui}
OUT=${OUT:-results}
RUNS=${1:-3}
WINID=${WINID:-}   # optional: a tool printing the window number of a process name, for `screencapture -l`
mkdir -p "$OUT"

cputime() { # ps "time" (m:ss.cc or h:mm:ss) -> seconds
  ps -o time= -p "$1" | awk -F: '{ if (NF==3) print $1*3600+$2*60+$3; else print $1*60+$2 }'
}

for run in $(seq 1 "$RUNS"); do
  for mode in idle play scroll; do
    LM_BENCH=$mode LM_BENCH_SECS=20 LM_BENCH_OUT="$OUT/run$run" "$BIN" 2>"$OUT/run$run-$mode.log" &
    pid=$!
    sleep 3
    c0=$(cputime $pid); t0=$(perl -MTime::HiRes=time -e "print time")
    if [ "$mode" = play ] && [ "$run" = 1 ] && [ -n "$WINID" ]; then
      sleep 5; id=$("$WINID" proto-gpui || true)
      [ -n "$id" ] && screencapture -x -o -l "$id" gpui/screenshot.png
      sleep 13
    else
      sleep 18
    fi
    c1=$(cputime $pid); t1=$(perl -MTime::HiRes=time -e "print time")
    rss=$(ps -o rss= -p $pid | tr -d ' ')
    fp=$(footprint -p $pid 2>/dev/null | awk '/phys_footprint:/ {print $2, $3; exit}')
    wait $pid || true
    cpu=$(echo "$c0 $c1 $t0 $t1" | awk '{ printf "%.1f", ($2-$1)/($4-$3)*100 }')
    echo "{\"run\":$run,\"mode\":\"$mode\",\"cpu_percent\":$cpu,\"rss_kb\":$rss,\"phys_footprint\":\"$fp\"}" \
      | tee -a "$OUT/process.jsonl"
  done
done
