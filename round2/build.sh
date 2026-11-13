#!/usr/bin/env bash
# build.sh <rev> <bench>: build a CQLite bench at <rev> into a pinned binary plus manifest.
#   <rev>   sha or branch name in $R2_CLONE;  <bench>  read | decode | partition_lookup
#   prints: <path> <sha256>
# Binary:   $R2_BINS/<sha12>/<bench>   Manifest: $R2_BINS/<sha12>/<bench>.meta.json
set -euo pipefail
. /home/dev/projects/machsmith/round2/env.sh

[ $# -eq 2 ] || { echo "usage: build.sh <rev> <bench>" >&2; exit 2; }
rev=$1 bench=$2
case "$bench" in read|decode|partition_lookup) ;; *) echo "build.sh: unknown bench $bench" >&2; exit 2 ;; esac
r2_require_unlocked || exit 1
command -v jq >/dev/null || { echo "build.sh: jq is required" >&2; exit 1; }

sha=$(git -C "$R2_CLONE" rev-parse --verify "$rev^{commit}") || { echo "build.sh: cannot resolve $rev in $R2_CLONE" >&2; exit 1; }
sha12=${sha:0:12}
features="cli-helpers,bench-internals"
wt=$R2_WT/$sha12

if [ ! -e "$wt" ]; then
    mkdir -p "$R2_WT"
    git -C "$R2_CLONE" worktree add --detach "$wt" "$sha" >&2
fi
[ "$(git -C "$wt" rev-parse HEAD)" = "$sha" ] || { echo "build.sh: $wt is not at $sha" >&2; exit 1; }

# The branch the caller named if <rev> is one, else the first local branch at that commit.
branch=""
if git -C "$R2_CLONE" show-ref --verify --quiet "refs/heads/$rev"; then branch=$rev
else branch=$(git -C "$R2_CLONE" for-each-ref --format='%(refname:short)' --points-at "$sha" refs/heads | head -n 1); fi

# One shared target dir for every revision (work/target/r2-bench).
json=$(cd "$wt" && nice -n 10 bash -c '. /home/dev/projects/machsmith/round2/env.sh
    msx_cargo r2-bench bench -p cqlite-core --features "$1" --bench "$2" --no-run --message-format=json-render-diagnostics' _ "$features" "$bench") \
    || { echo "build.sh: cargo bench --no-run failed" >&2; exit 1; }
exe=$(printf '%s\n' "$json" | jq -r --arg b "$bench" \
    'select(.reason=="compiler-artifact" and .executable!=null and .target.name==$b and (.target.kind|index("bench"))) | .executable' | tail -n 1)
[ -n "$exe" ] && [ -x "$exe" ] || { echo "build.sh: cargo reported no executable for bench $bench" >&2; exit 1; }

out=$R2_BINS/$sha12
mkdir -p "$out"
cp -f "$exe" "$out/$bench.tmp.$$"
mv -f "$out/$bench.tmp.$$" "$out/$bench"
bin_sha=$(sha256sum "$out/$bench" | cut -d' ' -f1)

jq -n --arg c "$sha" --arg br "$branch" --arg b "$bench" --arg f "$features" --arg s "$bin_sha" \
    --arg t "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    '{cqlite_commit:$c, branch:(if $br=="" then null else $br end), bench:$b, features:$f, sha256:$s, built_at:$t}' \
    > "$out/$bench.meta.json"

echo "$out/$bench $bin_sha"
