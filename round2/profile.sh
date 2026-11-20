#!/usr/bin/env bash
# profile.sh <bin> <workload> <tag>: per-case perf profile of a pinned bench binary (does not build).
#   <bin>  the cargo-bench-profile binary the referee measures (build.sh output, spec D5)
# Output: $MSX_WORK/r2-profile/<tag>/<workload>/<case-slug>.{data,script,mmap}, cases.json, host.json
# Env:    R2_PROFILE_CALLGRAPH=1  dwarf call graphs at -F 999, written under <tag>-cg/
#         R2_PROFILE_SHORT=1      first case only, --profile-time 2
#         R2_TEST_MODE=1          skip the quiet wait (host.json records test_mode true); short checks only
set -euo pipefail
. /home/dev/projects/machsmith/round2/env.sh

[ $# -eq 3 ] || { echo "usage: profile.sh <bin> <workload> <tag>" >&2; exit 2; }
bin=$1 workload=$2 tag=$3
[ -x "$bin" ] || { echo "profile.sh: $bin is not an executable" >&2; exit 1; }
case "$workload" in read|decode|partition_lookup) ;; *) echo "profile.sh: unknown workload $workload" >&2; exit 2 ;; esac
command -v jq >/dev/null || { echo "profile.sh: jq is required" >&2; exit 1; }
r2_require_unlocked || exit 1

test_mode=${R2_TEST_MODE:-0}
callgraph=${R2_PROFILE_CALLGRAPH:-0}
short=${R2_PROFILE_SHORT:-0}
r2bin=${R2_BIN:-$MSX_WORK/target/r2-tool/debug/r2}

# Profiling is a measurement: hold the lock for the whole run, then wait for a quiet host.
exec 9>"$R2_LOCK"
flock -n 9 || { echo "profile.sh: $R2_LOCK is held by a measurement run" >&2; exit 1; }

if [ "$test_mode" = 1 ]; then
    read -r l1 l5 l15 _ < /proc/loadavg
    snap=$(jq -n --argjson a "$(date +%s)" --argjson l1 "$l1" --argjson l5 "$l5" --argjson l15 "$l15" \
        '{at_unix:$a, load1:$l1, load5:$l5, load15:$l15, cargo_rustc_pids:[], core:null}')
else
    [ -x "$r2bin" ] || { echo "profile.sh: $r2bin missing; build r2_referee first" >&2; exit 1; }
    snap=$("$r2bin" quiet-wait)
fi
core=$(jq -r '.core // empty' <<<"$snap")
core_source=chosen
if [ -n "${R2_PROFILE_CORE:-}" ]; then
    core=$R2_PROFILE_CORE; core_source=R2_PROFILE_CORE
fi
[ -n "$core" ] || { [ -x "$r2bin" ] && core=$("$r2bin" choose-core); }
[ -n "$core" ] || { echo "profile.sh: cannot choose a core (r2 binary missing)" >&2; exit 1; }

if [ "$callgraph" = 1 ]; then
    tagdir="$tag-cg"; freq=999; cgargs=(--call-graph dwarf,16384)
else
    tagdir="$tag"; freq=4999; cgargs=()
fi
out=$MSX_WORK/r2-profile/$tagdir/$workload
mkdir -p "$out"
ptime=20; [ "$short" = 1 ] && ptime=2

jq --arg tag "$tagdir" --arg workload "$workload" --arg bin "$bin" --argjson test "$([ "$test_mode" = 1 ] && echo true || echo false)" \
    --argjson core "$core" --arg csrc "$core_source" --argjson freq "$freq" --argjson ptime "$ptime" --argjson cg "$([ "$callgraph" = 1 ] && echo true || echo false)" \
    --arg perf "$(perf --version)" --arg kernel "$(uname -r)" \
    '. + {tag:$tag, workload:$workload, bin:$bin, test_mode:$test, core:$core, core_source:$csrc, freq:$freq, profile_time:$ptime, callgraph:$cg, perf:$perf, kernel:$kernel}' \
    <<<"$snap" > "$out/host.json"

# Case ids: lines ending ": benchmark" (Criterion's --list), without the suffix.
mapfile -t ids < <("$bin" --list 2>/dev/null | sed -n 's/: benchmark$//p')
[ "${#ids[@]}" -gt 0 ] || { echo "profile.sh: $bin lists no benchmark cases" >&2; exit 1; }
[ "$short" = 1 ] && ids=("${ids[0]}")

escape_regex() { sed -e 's/[][\\.^$*+?(){}|]/\\&/g' <<<"$1"; }
slugify() { sed -e 's/[^A-Za-z0-9._-]/_/g' <<<"$1"; }

declare -A seen
cases='{}'
for id in "${ids[@]}"; do
    slug=$(slugify "$id")
    if [ -n "${seen[$slug]:-}" ]; then slug="$slug-$(printf '%s' "$id" | sha256sum | cut -c1-8)"; fi
    seen[$slug]=1
    cases=$(jq --arg s "$slug" --arg id "$id" '. + {($s): $id}' <<<"$cases")
    re="^$(escape_regex "$id")\$"
    echo "profile.sh: $slug ($ptime s, core $core)" >&2
    taskset -c "$core" perf record -e cycles:u -F "$freq" "${cgargs[@]}" -o "$out/$slug.data" -- \
        "$bin" --bench --profile-time "$ptime" "$re" >"$out/$slug.run.log" 2>&1
    perf script --no-demangle -i "$out/$slug.data" -F ip,sym,symoff,dso > "$out/$slug.script" 2>/dev/null
    # Load addresses of the executable mappings; attribute.py needs the libc bias to name stripped-libc functions.
    perf script --show-mmap-events -i "$out/$slug.data" -F ip 2>/dev/null | command grep 'PERF_RECORD_MMAP2.* r-xp ' \
        > "$out/$slug.mmap" || true
    echo "profile.sh: $slug: $(wc -l < "$out/$slug.script") script lines" >&2
done
printf '%s\n' "$cases" | jq -S . > "$out/cases.json"
echo "$out"
