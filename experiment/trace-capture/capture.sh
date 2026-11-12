#!/usr/bin/env bash
# Records vint and bti_sparse_find inputs from CQLite's own bench cases.
# Run from anywhere. Output: work/traces-raw/<bench>/<case-slug>/<kernel>.<seq>.bin
# and work/traces-raw/failed-cases.txt. Cases run sequentially (shared host).
set -u
ROOT=/home/dev/projects/machsmith
. "$ROOT/experiment/env.sh"
msx_require_env || exit 1
CLONE=$MSX_WORK/cqlite
PATCH=$ROOT/experiment/trace-capture/cqlite-trace.patch
RAW=$MSX_WORK/traces-raw
TARGET=$MSX_WORK/target/cqlite-trace
export CQLITE_DATASETS_ROOT=/home/dev/projects/cqlite/test-data/datasets
BENCHES="read partition_lookup m1_performance"

# 1. apply the patch to the clone (only the clone) unless already applied
if git -C "$CLONE" apply --reverse --check "$PATCH" >/dev/null 2>&1; then
    echo "patch already applied"
else
    git -C "$CLONE" apply "$PATCH" || { echo "patch failed" >&2; exit 1; }
fi

# 2. build
(cd "$CLONE" && CARGO_TARGET_DIR=$TARGET cargo bench -p cqlite-core --profile perfsym \
    --features cli-helpers,bench-internals \
    --bench read --bench partition_lookup --bench m1_performance --no-run) || exit 1

# 3. run each case once
rm -rf "$RAW"; mkdir -p "$RAW"
: > "$RAW/failed-cases.txt"
for b in $BENCHES; do
    bin=$(ls -t "$TARGET"/perfsym/deps/${b}-* 2>/dev/null | grep -v '\.d$' | head -1)
    [ -x "$bin" ] || { echo "no binary for $b" >&2; exit 1; }
    "$bin" --bench --list 2>/dev/null | sed -n 's/: benchmark$//p' > "$RAW/$b.cases"
    while IFS= read -r id; do
        [ -n "$id" ] || continue
        slug=$(printf '%s' "$id" | tr -c 'A-Za-z0-9._\n-' '_')
        dir=$RAW/$b/$slug
        mkdir -p "$dir"
        echo "== $b :: $id"
        printf '%s\n' "$id" > "$dir/ORIGIN"
        re=$(printf '%s' "$id" | sed 's/[][\.*^$(){}+?|]/\\&/g')
        if ! err=$(MSX_TRACE_DIR=$dir "$bin" --bench --test "^${re}\$" 2>&1 >/dev/null); then
            printf '%s\t%s\t%s\n' "$b" "$id" "$(printf '%s' "$err" | tail -3 | tr '\n' ' ')" >> "$RAW/failed-cases.txt"
            rm -rf "$dir"
        fi
    done < "$RAW/$b.cases"
done
echo "done; failures:"; cat "$RAW/failed-cases.txt"
