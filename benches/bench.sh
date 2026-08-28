#!/bin/sh
# Time the four things CtxC has to be fast at, against a fixed corpus.
#
#   sh benches/bench.sh [CORPUS]
#
# CORPUS defaults to this workspace's `crates`. Point it at something larger
# for numbers that mean more; the same corpus every time is what makes two runs
# comparable.
#
# Each run gets its own CTXC_HOME, so a warm database from the last run cannot
# flatter this one. Nothing here touches your real CtxC installation.
#
# There is no criterion here on purpose. These are whole-command timings — a
# cold index includes the walk, the parse, the writes and the process start,
# and that total is what someone waiting on `ctxc project index` experiences.

set -eu

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CORPUS="${1:-$ROOT/crates}"
RUNS="${CTXC_BENCH_RUNS:-3}"

if [ "${OS-}" = "Windows_NT" ]; then
    CTXC="$ROOT/target/release/ctxc.exe"
else
    CTXC="$ROOT/target/release/ctxc"
fi

[ -x "$CTXC" ] || {
    printf 'error: %s is missing. Run: cargo build --release -p ctxc-cli\n' "$CTXC" >&2
    exit 1
}
[ -d "$CORPUS" ] || {
    printf 'error: %s is not a directory\n' "$CORPUS" >&2
    exit 1
}

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT INT TERM

# Milliseconds since the epoch, from whatever this shell has. `date +%s%N` is
# GNU; the Python fallback covers the shells that do not have it.
now_ms() {
    stamp="$(date +%s%N 2>/dev/null || echo n)"
    case "$stamp" in
        *N|n) python3 -c 'import time; print(int(time.time()*1000))' ;;
        *) echo $((stamp / 1000000)) ;;
    esac
}

# Run a command RUNS times and report the fastest. The fastest run is the one
# least polluted by whatever else the machine was doing.
best() {
    label="$1"
    shift
    lowest=""
    index=0
    while [ "$index" -lt "$RUNS" ]; do
        start="$(now_ms)"
        "$@" >/dev/null 2>&1 || true
        elapsed=$(($(now_ms) - start))
        if [ -z "$lowest" ] || [ "$elapsed" -lt "$lowest" ]; then
            lowest="$elapsed"
        fi
        index=$((index + 1))
    done
    printf '  %-34s %6s ms   (best of %s)\n' "$label" "$lowest" "$RUNS"
}

# A fresh home, so "cold" means cold.
fresh_home() {
    rm -rf "$WORK/home"
    mkdir -p "$WORK/home"
}

cold_index() {
    fresh_home
    CTXC_HOME="$WORK/home" "$CTXC" project index "$CORPUS"
}

printf 'CtxC benchmark\n'
printf '  binary  %s\n' "$CTXC"
printf '  corpus  %s\n\n' "$CORPUS"

printf 'Indexing\n'
best 'cold index' cold_index

# One warm home, indexed once, reused by everything below: a warm re-index and
# a search both need an index that is already there.
fresh_home
CTXC_HOME="$WORK/home" "$CTXC" project index "$CORPUS" >/dev/null 2>&1
export CTXC_HOME="$WORK/home"

best 'warm re-index (nothing changed)' "$CTXC" project index "$CORPUS"

printf '\nSearch\n'
best 'find' "$CTXC" find 'index the project' --path "$CORPUS"
best 'find --compile' "$CTXC" find 'index the project' --path "$CORPUS" --compile

# One megabyte of log lines that repeat with small variations, which is what
# the log optimizer exists for. Generated rather than committed: a fixed shape
# beats a fixed file nobody can regenerate.
printf '\nOptimize\n'
index=0
: > "$WORK/big.log"
while [ "$index" -lt 12000 ]; do
    printf '2024-01-01T00:%02d:%02d INFO  worker %d handled request %d in %d ms\n' \
        $((index % 60)) $((index % 60)) $((index % 8)) "$index" $((index % 97)) \
        >> "$WORK/big.log"
    index=$((index + 1))
done
printf '  %-34s %6s\n' 'corpus' "$(wc -c < "$WORK/big.log") bytes"

best 'optimize a 1 MB log' "$CTXC" optimize "$WORK/big.log" --no-cache
best 'optimize it again (cached)' "$CTXC" optimize "$WORK/big.log"
