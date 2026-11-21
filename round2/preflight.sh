#!/usr/bin/env bash
# preflight.sh: run before any measurement. Checks, in order, stopping at the first failure with one
# stderr line `preflight: <reason>` and a non-zero exit: perf via the shim, kernel (informational),
# lock free, pinned CQLITE_DATASETS_ROOT, binaries.json files and sha256s, r2 binary.
# Success: last stdout line is `preflight: ok`, exit 0.
#
# Test-only overrides (applied after env.sh is sourced): R2_PREFLIGHT_BINARIES (binaries.json path),
# R2_PREFLIGHT_ROOT (base dir for its relative paths), R2_PREFLIGHT_LOCK (lock path), R2_BIN, R2_PERF.
pinned_datasets=/home/dev/projects/cqlite/test-data/datasets
caller_datasets=${CQLITE_DATASETS_ROOT-}   # env.sh overwrites it; capture first
caller_datasets_set=${CQLITE_DATASETS_ROOT+1}

. /home/dev/projects/machsmith/round2/env.sh

fail() { echo "preflight: $*" >&2; exit 1; }

binaries=${R2_PREFLIGHT_BINARIES:-$R2_ROOT/results/profile/binaries.json}
broot=${R2_PREFLIGHT_ROOT:-$MSX_ROOT}
[ -z "${R2_PREFLIGHT_LOCK:-}" ] || R2_LOCK=$R2_PREFLIGHT_LOCK
r2bin=${R2_BIN:-$MSX_WORK/target/r2-tool/debug/r2}

# 1. perf through the shim
perf_path=$(command -v perf) || fail "perf not found on PATH"
[ "$perf_path" = "$R2_ROOT/bin/perf" ] || fail "perf resolves to $perf_path, not $R2_ROOT/bin/perf"
perf_ver=$(perf --version 2>&1) || fail "perf --version failed (R2_PERF=${R2_PERF:-unset}): $(head -n1 <<<"$perf_ver")"
echo "perf: $(head -n1 <<<"$perf_ver")"

# 2. kernel
echo "kernel: $(uname -r)"

# 3. lock
r2_require_unlocked 2>/dev/null || fail "$R2_LOCK is held by a measurement run"

# 4. datasets root
if [ -n "$caller_datasets_set" ] && [ "$caller_datasets" != "$pinned_datasets" ]; then
    fail "CQLITE_DATASETS_ROOT is '$caller_datasets' in the environment, pinned value is '$pinned_datasets'"
fi
[ "${CQLITE_DATASETS_ROOT:-}" = "$pinned_datasets" ] || fail "CQLITE_DATASETS_ROOT is '${CQLITE_DATASETS_ROOT:-}', pinned value is '$pinned_datasets'"
[ -d "$pinned_datasets" ] || fail "datasets root $pinned_datasets is not a directory"

# 5. pinned binaries
[ -f "$binaries" ] || fail "$binaries not found"
entries=$(jq -r 'to_entries[] | [.key, .value.path, .value.sha256] | @tsv' "$binaries" 2>/dev/null) || fail "cannot parse $binaries"
while IFS=$'\t' read -r name path want; do
    [ -n "$name" ] || continue
    [ -f "$broot/$path" ] || fail "binary for $name missing: $broot/$path"
    got=$(sha256sum "$broot/$path" | cut -d' ' -f1)
    [ "$got" = "$want" ] || fail "sha256 mismatch for $name ($broot/$path): got $got, want $want"
done <<<"$entries"

# 6. r2
[ -f "$r2bin" ] && [ -x "$r2bin" ] || fail "r2 binary $r2bin missing or not executable"

echo "preflight: ok"
