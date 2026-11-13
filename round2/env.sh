# Sourced by every round 2 script. Do not execute.
# experiment/env.sh unsets CARGO_TARGET_DIR (the shared one must never be used).
. /home/dev/projects/machsmith/experiment/env.sh
export R2_ROOT=$MSX_ROOT/round2
export R2_CLONE=$MSX_WORK/cqlite-r2
export R2_WT=$MSX_WORK/cqlite-r2-wt
export R2_BINS=$MSX_WORK/r2-bins
export R2_LOCK=$MSX_WORK/r2.lock
export R2_LEDGER=$R2_ROOT/results/ledger.jsonl
export CQLITE_DATASETS_ROOT=/home/dev/projects/cqlite/test-data/datasets
export R2_BASE=7f9abe3663254724439dbe71aff77bb93faa1c2d

# Returns 1 with a message when a measurement run holds $R2_LOCK.
r2_require_unlocked() {
    [ -e "$R2_LOCK" ] || return 0
    if flock -n "$R2_LOCK" true 2>/dev/null; then
        return 0
    fi
    echo "r2: $R2_LOCK is held by a measurement run; do not build or test now" >&2
    return 1
}
