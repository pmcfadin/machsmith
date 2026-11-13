#!/usr/bin/env bash
# test.sh <rev>: run CQLite's test suite on <rev> in $R2_WT/<sha12>, wait for it to finish, and
# append a kind "tests" ledger record. Mode "gate": CQLite's scripts/agent-gate.sh via
# scripts/flow/gate-detached.sh (docs/development/gate-ops.md; completion = gate-liveness.sh exit 0,
# verdict = the RESULT: PASS|FAIL token). Mode "fallback": cargo test + clippy, used only when the
# gate cannot certify because the tree is a clone (behind origin/main, tree integrity, refusal).
# Exit status: 0 only if the suite passed.
set -uo pipefail
. /home/dev/projects/machsmith/round2/env.sh

# decide <RESULT token> <component-set: line> <tree-integrity: line>  ->  pass | fallback | fail
# fallback ONLY for a full-gate FAIL whose component-set line is FAIL-CLOSED because the clone is
# BEHIND origin/main (agent-gate.sh ~6802), the ancestry probe is indeterminate (~6810), or the baseline is
# unmeasured because origin is not the canonical upstream (remote-not-canonical). Every other
# outcome (tree-integrity FAIL, other FAIL-CLOSED causes, ERROR/REFUSED, unknown tokens) is a recorded failure.
decide() {
    local result=$1 cs=$2 ti=$3
    case "$ti" in "tree-integrity: FAIL"*) echo fail; return ;; esac
    case "$result" in
        PASS) echo pass ;;
        FAIL)
            case "$cs" in
                "component-set: FAIL-CLOSED"*"this tree is BEHIND origin/main"*|\
                "component-set: FAIL-CLOSED"*"ancestry probe could not tell BEHIND from a deliberate removal"*)
                    echo fallback ;;
                "component-set: FAIL-CLOSED"*"remote-not-canonical"*) echo fallback ;;   # clone's origin is local, not the canonical upstream
                *) echo fail ;;
            esac ;;
        *) echo fail ;;
    esac
}
# launch_runid <launch-output-file>: the run-id from the "--run-id <id>" in gate-detached's poll line.
launch_runid() { sed -n 's/.*gate-liveness\.sh .* --run-id \([^[:space:]]*\).*/\1/p' "$1" 2>/dev/null | head -n 1; }
# step_json <test-log> <exit-code> <log-path>: {exit, passed, failed, failing_tests, build_failed, log}.
# Sums every `test result:` line; failing names from `---- <name> stdout ----` headers and the
# `failures:` lists (4-space-indented names). build_failed = exit != 0 with 0 tests counted and no names.
step_json() {
    local log=$1 rc=$2 p=0 f=0 n names
    for n in $(sed -n 's/^test result: .* \([0-9][0-9]*\) passed;.*/\1/p' "$log" 2>/dev/null); do p=$((p + n)); done
    for n in $(sed -n 's/^test result: .* \([0-9][0-9]*\) failed;.*/\1/p' "$log" 2>/dev/null); do f=$((f + n)); done
    names=$( { sed -n 's/^---- \(.*\) stdout ----$/\1/p' "$log" 2>/dev/null
               awk '/^failures:$/{on=1;next} on&&/^    [^ ]/{sub(/^    /,"");print;next} on&&!/^$/{on=0}' "$log" 2>/dev/null; } \
             | sort -u | jq -R . | jq -s .)
    jq -n --argjson rc "$rc" --argjson p "$p" --argjson f "$f" --argjson names "$names" --arg log "$3" \
        '{exit:$rc, passed:$p, failed:$f, failing_tests:$names, log:$log,
          build_failed:($rc != 0 and $p == 0 and $f == 0 and ($names|length) == 0)}'
}
# free_gib <path>: available GiB on the volume holding <path> (empty if unknown).
free_gib() { df -BG --output=avail "$1" 2>/dev/null | tail -n 1 | tr -dc '0-9'; }
# need_free <path> <min-gib> <label>: returns 1 with a message when the volume has less than <min-gib> free.
need_free() {
    local have; have=$(free_gib "$1")
    if [ -z "$have" ] || [ "$have" -lt "$2" ]; then
        echo "test.sh: refusing to start: $3 ($1) has ${have:-unknown} GiB available, need >= $2 GiB" >&2
        return 1
    fi
}
# vol_json <target-dir>: disk figures to attach to a step record.
vol_json() {
    jq -n --argjson du "$(du -sx --block-size=1M "$1" 2>/dev/null | cut -f1 | command grep -E '^[0-9]+$' || echo null)" \
        --argjson ft "$(free_gib "$1" | command grep -E '^[0-9]+$' || echo null)" \
        --argjson fp "$(free_gib /home/dev/projects | command grep -E '^[0-9]+$' || echo null)" \
        '{target_dir_mib:$du, free_gib_target_volume:$ft, free_gib_projects_volume:$fp}'
}
[ "${TESTSH_SOURCE_ONLY:-}" = 1 ] && return 0 2>/dev/null

[ $# -eq 1 ] || { echo "usage: test.sh <rev>" >&2; exit 2; }
r2_require_unlocked || exit 1
command -v jq >/dev/null || { echo "test.sh: jq is required" >&2; exit 1; }

# Build outputs live on the dev cache volume (development-environment.md), never in work/.
tdir=/home/dev/.cache-dev/targets/machsmith-r2/r2-test
mkdir -p "$tdir" || exit 1
need_free "$tdir" "${R2_TEST_MIN_FREE_GIB:-60}" "target volume" || exit 1
need_free /home/dev/projects "${R2_TEST_MIN_PROJECTS_FREE_GIB:-10}" "projects volume" || exit 1
mkdir -p "$MSX_WORK/target"
[ -e "$MSX_WORK/target/r2-test" ] || [ -L "$MSX_WORK/target/r2-test" ] || ln -s "$tdir" "$MSX_WORK/target/r2-test"

sha=$(git -C "$R2_CLONE" rev-parse --verify "$1^{commit}") || { echo "test.sh: cannot resolve $1 in $R2_CLONE" >&2; exit 1; }
sha12=${sha:0:12}
wt=$R2_WT/$sha12-test   # test.sh has its own worktree so build.sh can never dirty the gate's tree
if [ ! -e "$wt" ]; then
    mkdir -p "$R2_WT"
    git -C "$R2_CLONE" worktree add --detach "$wt" "$sha" >&2 || exit 1
fi
[ "$(git -C "$wt" rev-parse HEAD)" = "$sha" ] || { echo "test.sh: $wt is not at $sha" >&2; exit 1; }

export CARGO_TARGET_DIR=$tdir
gatedir=$MSX_WORK/r2-gate
mkdir -p "$gatedir"
summary=$gatedir/$sha12.summary
log=$gatedir/$sha12.log
rm -f "$summary" "$summary.heartbeat" "$log"
body=$(mktemp "$gatedir/$sha12.body.XXXXXX.json")
fb=0 recorded=0 steps_so_far='{}'
on_exit() {
    local rc=$?
    if [ "$fb" = 1 ] && [ "$recorded" = 0 ]; then
        echo "test.sh: unexpected exit ($rc) during the fallback; recording ABORTED" >&2
        jq -n --arg rev "$sha" --arg why "${cs:-} ${ti:-}" --argjson rc "$rc" --argjson steps "$steps_so_far" --argjson vol "$(vol_json "$tdir")" \
            '{rev:$rev, mode:"fallback", result:"ABORTED", exit_status:1, exit_code:$rc, gate_why:$why, steps:$steps, volumes:$vol}' >"$body" 2>/dev/null \
            && "$r2bin" ledger-append --kind tests --body-file "$body" >&2
    fi
    rm -f "$body"
}
trap on_exit EXIT
trap 'exit 143' TERM
trap 'exit 130' INT

# Always bring the r2 binary up to date now, and prove ledger-append exists before a multi-hour gate.
(cd "$R2_ROOT" && nice -n 10 bash -c '. /home/dev/projects/machsmith/round2/env.sh; msx_cargo r2-tool build -p r2_referee --bin r2') >&2 || { echo "test.sh: cannot build r2" >&2; exit 1; }
r2bin=$MSX_WORK/target/r2-tool/debug/r2
probe=$("$r2bin" ledger-append --kind probe --body-file /dev/null 2>&1 >/dev/null)
case "$probe" in *"only --kind tests"*) ;; *) echo "test.sh: $r2bin lacks a working ledger-append: $probe" >&2; exit 1 ;; esac

export CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0   # no debuginfo: forwarded to the gate and used by the fallback
mode=gate status=0 gate_rc=0
cd "$wt" || exit 1
echo "test.sh: launching the gate detached in $wt (summary $summary)" >&2
nice -n 10 bash scripts/flow/gate-detached.sh --summary "$summary" --log "$log" >"$gatedir/$sha12.launch" 2>&1
gate_rc=$?
result=""
if [ "$gate_rc" -ne 0 ]; then
    cat "$gatedir/$sha12.launch" >&2
    echo "test.sh: gate launch refused (exit $gate_rc)" >&2
    result="LAUNCH_FAILED"
else
    runid=$(launch_runid "$gatedir/$sha12.launch")
    # gate-detached may return before the summary exists: poll for a run-id line, 2 s steps, 300 s max.
    waited=0
    while [ -z "$runid" ] && [ "$waited" -lt 300 ]; do
        runid=$(sed -n 's/^run-id:[[:space:]]*//p' "$summary" 2>/dev/null | head -n 1)
        [ -n "$runid" ] || { sleep 2; waited=$((waited + 2)); }
    done
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

cs="" ti=""
if [ -f "$summary" ]; then
    cs=$(command grep -E '^component-set:' "$summary" | head -n 1)
    ti=$(command grep -E '^tree-integrity:' "$summary" | head -n 1)
fi
decision=$(decide "$result" "$cs" "$ti")
case "$decision" in pass|fallback|fail) ;; *) decision=fail ;; esac
case "$result" in PASS|FAIL|LAUNCH_FAILED|NO_RUN_ID|LIVENESS_LOST|NO_RESULT) ;; *) echo "test.sh: unknown terminal result '$result'; recording a failure (no fallback)" >&2 ;; esac

if [ "$decision" = fallback ]; then
    mode=fallback fb=1
    echo "test.sh: gate cannot certify this clone (result=$result; cause: ${cs:-no component-set line}); running fallback cargo test + clippy" >&2
    wlog=$gatedir/$sha12.fallback-workspace.log
    klog=$gatedir/$sha12.fallback-core.log
    clog=$gatedir/$sha12.fallback-clippy.log
    nice -n 10 cargo test --workspace --no-fail-fast --exclude cqlite-py --exclude cqlite-node >"$wlog" 2>&1
    w_rc=$?
    wj=$(jq -s add <(step_json "$wlog" "$w_rc" "$wlog") <(vol_json "$tdir")); steps_so_far=$(jq -n --argjson w "$wj" '{workspace:$w}')
    nice -n 10 cargo test --no-fail-fast -p cqlite-core --features cli-helpers >"$klog" 2>&1
    k_rc=$?
    kj=$(jq -s add <(step_json "$klog" "$k_rc" "$klog") <(vol_json "$tdir")); steps_so_far=$(jq -n --argjson w "$wj" --argjson k "$kj" '{workspace:$w, core_cli_helpers:$k}')
    RUSTFLAGS="-D warnings" nice -n 10 cargo clippy --workspace --all-targets --all-features >"$clog" 2>&1
    clippy_rc=$?
    cj=$(jq -s add <(jq -n --argjson rc "$clippy_rc" --arg log "$clog" '{exit:$rc, log:$log}') <(vol_json "$tdir"))
    [ "$w_rc" -eq 0 ] && [ "$k_rc" -eq 0 ] && [ "$clippy_rc" -eq 0 ] && status=0 || status=1
    jq -n --arg rev "$sha" --arg gate_result "$result" --arg why "${cs:-} ${ti:-}" \
        --argjson w "$wj" --argjson k "$kj" --argjson c "$cj" --argjson clippy_rc "$clippy_rc" --argjson status "$status" \
        '{rev:$rev, mode:"fallback", gate_result:$gate_result, gate_why:$why,
          steps:{workspace:$w, core_cli_helpers:$k, clippy:$c},
          tests_passed:($w.passed+$k.passed), tests_failed:($w.failed+$k.failed),
          failing_tests:(($w.failing_tests+$k.failing_tests)|unique),
          cargo_test_exit:(if $w.exit!=0 then $w.exit else $k.exit end), clippy_exit:$clippy_rc,
          exit_status:$status}' >"$body"
else
    [ "$decision" = pass ] && status=0 || status=1
    jq -n --arg rev "$sha" --arg mode gate --arg result "$result" --argjson status "$status" \
        --rawfile summary "$summary" --argjson gate_rc "$gate_rc" --arg log "$log" --arg why "${cs:-} ${ti:-}" \
        '{rev:$rev, mode:$mode, result:$result, gate_why:$why, summary:$summary, gate_launch_exit:$gate_rc,
          exit_status:$status, log:$log}' >"$body" 2>/dev/null \
    || jq -n --arg rev "$sha" --arg result "$result" --argjson status "$status" \
        '{rev:$rev, mode:"gate", result:$result, summary:null, exit_status:$status}' >"$body"
fi

id=$("$r2bin" ledger-append --kind tests --body-file "$body") || { echo "test.sh: ledger append failed" >&2; exit 1; }
recorded=1
echo "test.sh: $sha12 mode=$mode result=${result:-n/a} exit_status=$status ledger=$id"
exit "$status"
