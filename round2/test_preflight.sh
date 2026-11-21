#!/usr/bin/env bash
# Tests for preflight.sh using temp-dir fixtures. Quick; no measurement.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
PRE=$HERE/preflight.sh
T=$(mktemp -d); trap 'kill $(jobs -p) 2>/dev/null; rm -rf "$T"' EXIT
fails=0

good_perf=$T/perf-ok;  printf '#!/bin/sh\necho "perf version 9.9.9"\n' > "$good_perf"
bad_perf=$T/perf-bad;  printf '#!/bin/sh\nexit 1\n' > "$bad_perf"
good_r2=$T/r2;         printf '#!/bin/sh\nexit 0\n' > "$good_r2"
noexec_r2=$T/r2-noexec; printf '#!/bin/sh\n' > "$noexec_r2"
chmod +x "$good_perf" "$bad_perf" "$good_r2"
mkdir -p "$T/root/bins"
echo hello > "$T/root/bins/read"
sha=$(sha256sum "$T/root/bins/read" | cut -d' ' -f1)
printf '{"read":{"path":"bins/read","sha256":"%s"}}\n' "$sha" > "$T/good.json"
printf '{"read":{"path":"bins/read","sha256":"%064d"}}\n' 0 > "$T/badsha.json"
printf '{"read":{"path":"bins/gone","sha256":"%s"}}\n' "$sha" > "$T/missing.json"

# run [VAR=value ...]; sets $rc $out $err
run() {
    env "R2_PREFLIGHT_BINARIES=$T/good.json" "R2_PREFLIGHT_ROOT=$T/root" "R2_PREFLIGHT_LOCK=$T/lock" \
        "R2_BIN=$good_r2" "R2_PERF=$good_perf" "$@" bash "$PRE" >"$T/out" 2>"$T/err"
    rc=$?; out=$(cat "$T/out"); err=$(cat "$T/err")
}
ok() { echo "PASS: $1"; }
bad() { echo "FAIL: $1 ($2)"; fails=$((fails+1)); }
expect_fail() {
    local name=$1 pat=${2:-}
    if [ "$rc" -eq 0 ]; then bad "$name" "exit 0"; return; fi
    if [ "$(printf '%s\n' "$err" | wc -l)" -ne 1 ] || [ -z "$err" ] || [[ $err != "preflight: "* ]]; then
        bad "$name" "stderr: $err"; return; fi
    if [ -n "$pat" ] && [[ $err != *$pat* ]]; then bad "$name" "stderr lacks '$pat': $err"; return; fi
    ok "$name"
}

run
if [ "$rc" -eq 0 ] && [[ $out == *"perf: perf version 9.9.9"* ]] && [[ $out == *"kernel: $(uname -r)"* ]] \
   && [ "$(tail -n1 <<<"$out")" = "preflight: ok" ] && [ -z "$err" ]; then ok "all good"; else bad "all good" "rc=$rc out=$out err=$err"; fi

run "R2_PREFLIGHT_BINARIES=$T/badsha.json";  expect_fail "sha mismatch" "sha256"
run "R2_PREFLIGHT_BINARIES=$T/missing.json"; expect_fail "missing binary" "bins/gone"
run "R2_PERF=$bad_perf";                     expect_fail "broken perf" "perf"
run "R2_BIN=$T/nonexistent-r2";              expect_fail "missing r2" "r2"
run "R2_BIN=$noexec_r2";                     expect_fail "non-exec r2" "r2"
run "CQLITE_DATASETS_ROOT=/somewhere/else";  expect_fail "wrong datasets root" "/somewhere/else"

( exec 8>"$T/lock"; flock -n 8 && sleep 30 ) &
sleep 0.3
run;                                         expect_fail "held lock" "held"
kill $! 2>/dev/null; wait $! 2>/dev/null

[ "$fails" -eq 0 ] && echo "all passed" || { echo "$fails failed"; exit 1; }
