#!/usr/bin/env bash
# Usage: tools/profile-cqlite.sh <bench>   (read | partition_lookup | decode)
# Builds the CQLite bench with the perfprof profile and records user-space
# cycles with perf, pinned to one core. Output: work/profile/<bench>.script
set -euo pipefail
bench=${1:?usage: profile-cqlite.sh <bench>}
here=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
. "$here/experiment/env.sh"
msx_require_env || exit 1

export CARGO_TARGET_DIR="$MSX_WORK/target/cqlite-perfprof"
export CQLITE_DATASETS_ROOT=/home/dev/projects/cqlite/test-data/datasets
core=${MSX_PROFILE_CORE:-5}
out="$MSX_WORK/profile"
mkdir -p "$out"

( cd "$MSX_WORK/cqlite" && cargo bench -p cqlite-core --profile perfprof \
    --features cli-helpers,bench-internals --bench "$bench" --no-run )

# Newest executable named <bench>-<hash> (no extension) in deps.
bin=$(find "$CARGO_TARGET_DIR/perfprof/deps" -maxdepth 1 -type f -name "$bench-*" -executable \
        ! -name '*.d' -printf '%T@ %p\n' | sort -rn | head -1 | cut -d' ' -f2-)
[ -n "$bin" ] || { # Load addresses of the executable mappings; ceiling.py needs the libc bias to
# name stripped-libc functions.
perf script --show-mmap-events -i "$out/$bench.data" -F ip 2>/dev/null | grep 'PERF_RECORD_MMAP2.* r-xp ' \
    > "$out/$bench.mmap" || true
echo "profile-cqlite: no binary for $bench" >&2; exit 1; }
echo "$bin" > "$out/$bench.binpath"

taskset -c "$core" perf record -e cycles:u -F 4999 -o "$out/$bench.data" -- \
    "$bin" --bench --profile-time 4 >"$out/$bench.run.log" 2>&1
perf script --no-demangle -i "$out/$bench.data" -F ip,sym,symoff,dso > "$out/$bench.script"
# Load addresses of the executable mappings; ceiling.py needs the libc bias to
# name stripped-libc functions.
perf script --show-mmap-events -i "$out/$bench.data" -F ip 2>/dev/null | grep 'PERF_RECORD_MMAP2.* r-xp ' \
    > "$out/$bench.mmap" || true
echo "profile-cqlite: $(wc -l < "$out/$bench.script") samples -> $out/$bench.script"
