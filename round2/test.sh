#!/usr/bin/env bash
# test.sh <rev>: run CQLite's test suite on <rev> in $R2_WT/<sha12>, wait for it to finish, and
# append a kind "tests" ledger record. Mode "gate": CQLite's scripts/agent-gate.sh via
# scripts/flow/gate-detached.sh (docs/development/gate-ops.md; completion = gate-liveness.sh exit 0,
# verdict = the RESULT: PASS|FAIL token). Mode "fallback": cargo test + clippy, used only when the
# gate cannot certify because the tree is a clone (behind origin/main, tree integrity, refusal).
# Exit status: 0 only if the suite passed.
set -uo pipefail
. /home/dev/projects/machsmith/round2/env.sh

[ $# -eq 1 ] || { echo "usage: test.sh <rev>" >&2; exit 2; }
r2_require_unlocked || exit 1
command -v jq >/dev/null || { echo "test.sh: jq is required" >&2; exit 1; }

sha=$(git -C "$R2_CLONE" rev-parse --verify "$1^{commit}") || { echo "test.sh: cannot resolve $1 in $R2_CLONE" >&2; exit 1; }
sha12=${sha:0:12}
wt=$R2_WT/$sha12
if [ ! -e "$wt" ]; then
    mkdir -p "$R2_WT"
    git -C "$R2_CLONE" worktree add --detach "$wt" "$sha" >&2 || exit 1
fi
[ "$(git -C "$wt" rev-parse HEAD)" = "$sha" ] || { echo "test.sh: $wt is not at $sha" >&2; exit 1; }

export CARGO_TARGET_DIR=$MSX_WORK/target/r2-test
gatedir=$MSX_WORK/r2-gate
mkdir -p "$gatedir"
summary=$gatedir/$sha12.summary
log=$gatedir/$sha12.log
rm -f "$summary" "$summary.heartbeat" "$log"
body=$(mktemp "$gatedir/$sha12.body.XXXXXX.json")
trap 'rm -f "$body"' EXIT

r2bin=$MSX_WORK/target/r2-tool/debug/r2
if [ ! -x "$r2bin" ]; then
    (cd "$R2_ROOT" && msx_cargo r2-tool build -p r2_referee --bin r2) >&2 || exit 1
fi

mode=gate status=0 gate_rc=0
cd "$wt" || exit 1
echo "test.sh: launching the gate detached in $wt (summary $summary)" >&2
nice -n 10 bash scripts/flow/gate-detached.sh --summary "$summary" --log "$log" >"$gatedir/$sha12.launch" 2>&1
gate_rc=$?
cannot_certify=0
result=""
if [ "$gate_rc" -ne 0 ]; then
    cat "$gatedir/$sha12.launch" >&2
    echo "test.sh: gate launch refused (exit $gate_rc)" >&2
    result="LAUNCH_FAILED"
else
    runid=$(sed -n 's/^run-id:[[:space:]]*//p' "$summary" | head -n 1)
    [ -n "$runid" ] || { echo "test.sh: no run-id in $summary" >&2; result="NO_RUN_ID"; }
    bad=0
    while [ -n "$runid" ]; do
        bash scripts/gate-liveness.sh "$summary" --run-id "$runid" >"$gatedir/$sha12.liveness" 2>&1
        rc=$?
        case $rc in
            0) break ;;
            2) bad=0 ;;
            *) bad=$((bad + 1))
               if [ "$bad" -ge 10 ]; then result="LIVENESS_LOST"; break; fi ;;
        esac
        sleep 60
    done
    if [ -z "$result" ]; then
        result=$(command grep -E '^RESULT: ' "$summary" | tail -n 1 | sed 's/^RESULT: //; s/[[:space:]].*//')
        [ -n "$result" ] || result="NO_RESULT"
    fi
fi

if [ -f "$summary" ]; then
    # Reasons caused by being a clone: behind origin/main (component-set), tree integrity,
    # an outright refusal/error. A genuine FAIL with PASS integrity is a test failure, not this.
    cs=$(command grep -E '^component-set:' "$summary" | head -n 1)
    ti=$(command grep -E '^tree-integrity:' "$summary" | head -n 1)
    case "$cs" in *FAIL-CLOSED*|*origin/main*REFUSED*) [ "$result" = PASS ] || cannot_certify=1 ;; esac
    case "$ti" in "tree-integrity: FAIL"*) cannot_certify=1 ;; esac
fi
case "$result" in ERROR|REFUSED) cannot_certify=1 ;; esac

if [ "$cannot_certify" = 1 ]; then
    mode=fallback
    echo "test.sh: gate cannot certify this clone (result=$result); running fallback cargo test + clippy" >&2
    tlog=$gatedir/$sha12.fallback-test.log
    clog=$gatedir/$sha12.fallback-clippy.log
    nice -n 10 cargo test --workspace --no-fail-fast >"$tlog" 2>&1
    test_rc=$?
    RUSTFLAGS="-D warnings" nice -n 10 cargo clippy --workspace --all-targets --all-features >"$clog" 2>&1
    clippy_rc=$?
    passed=$(sed -n 's/^test result: .* \([0-9][0-9]*\) passed;.*/\1/p' "$tlog" | paste -sd+ - | bc 2>/dev/null)
    failed=$(sed -n 's/^test result: .* \([0-9][0-9]*\) failed;.*/\1/p' "$tlog" | paste -sd+ - | bc 2>/dev/null)
    names=$(sed -n 's/^test \(.*\) \.\.\. FAILED$/\1/p' "$tlog" | jq -R . | jq -s .)
    [ "$test_rc" -eq 0 ] && [ "$clippy_rc" -eq 0 ] && status=0 || status=1
    jq -n --arg rev "$sha" --arg mode fallback --arg gate_result "$result" --arg why "${cs:-} ${ti:-}" \
        --argjson passed "${passed:-0}" --argjson failed "${failed:-0}" --argjson names "$names" \
        --argjson test_rc "$test_rc" --argjson clippy_rc "$clippy_rc" --argjson status "$status" \
        --arg tlog "$tlog" --arg clog "$clog" \
        '{rev:$rev, mode:$mode, gate_result:$gate_result, gate_why:$why, tests_passed:$passed, tests_failed:$failed,
          failing_tests:$names, cargo_test_exit:$test_rc, clippy_exit:$clippy_rc, exit_status:$status,
          logs:[$tlog,$clog]}' >"$body"
else
    [ "$result" = PASS ] && status=0 || status=1
    jq -n --arg rev "$sha" --arg mode gate --arg result "$result" --argjson status "$status" \
        --rawfile summary "$summary" --argjson gate_rc "$gate_rc" --arg log "$log" \
        '{rev:$rev, mode:$mode, result:$result, summary:$summary, gate_launch_exit:$gate_rc,
          exit_status:$status, log:$log}' >"$body" 2>/dev/null \
    || jq -n --arg rev "$sha" --arg result "$result" --argjson status "$status" \
        '{rev:$rev, mode:"gate", result:$result, summary:null, exit_status:$status}' >"$body"
fi

id=$("$r2bin" ledger-append --kind tests --body-file "$body") || { echo "test.sh: ledger append failed" >&2; exit 1; }
echo "test.sh: $sha12 mode=$mode result=${result:-n/a} exit_status=$status ledger=$id"
exit "$status"
