# round2

Round 2 of machsmith: a referee and tooling for finding and measuring CQLite
hot spots, built on the round 1 experiment's statistics. Round 1 artifacts
(`experiment/`, `tools/`) are read only; round 2 copies what it reuses.

- Spec: `docs/specs/2026-11-12-round-2-design.md`
- Plan: `docs/plans/2026-11-12-round-2/` (start with its `README.md`)
- Decisions: `DECISIONS.md` (append only)
- Results: `results/` (`ledger.jsonl` is append-only; never edit it)

The CQLite clone lives at `work/cqlite-r2`, pinned to commit
`7f9abe3663254724439dbe71aff77bb93faa1c2d` (`results/clone.json`).

## Usage

```
. round2/env.sh
cd round2 && msx_cargo r2-tool build -p r2_referee
```

`env.sh` unsets `CARGO_TARGET_DIR`; `msx_cargo <name>` builds into
`work/target/<name>`. Never build into the shared cargo target dir, and never
build or test while `work/r2.lock` is held (`r2_require_unlocked`).

## `r2`, the referee

```
r2 quiet-wait [--hold-min 10] [--max-load 2.0] [--timeout-h 12]
r2 run --kind <null|calibration|screen|confirm|crosscheck|proposal> --workload <read|decode|partition_lookup>
       --base <bin> --cand <bin> [--sessions N] [--targets <file.json>] [--rule <id>] [--null <ledger-id>]
r2 report <ledger-id>
r2 nullband --null <ledger-id> --workload W [--targets <file.json>]
```

The ledger is `--ledger` or `$R2_LEDGER`; the run lock is `$R2_LOCK`; per-session
Criterion homes, perf output and `progress.json` go to `$R2_SCRATCH` (default a new
temp dir). A targets file is a JSON array of Criterion `full_id`s. Every run after
the lock appends exactly one ledger record with `status` `OK`, `NOISE_GATE_FAILED`,
`NULL_MISMATCH` (null's case set differs; no verdict), `aborted` (SIGINT/SIGTERM,
panic or tool error) or `refused`; outside test mode each binary needs its
`<bin>.meta.json` build manifest and `CQLITE_DATASETS_ROOT` must be the pinned root; `r2 run` exits 0 only
for `OK`. The per-session noise gate rejects on load, cargo/rustc, SMT-sibling use, or
> 2% of an arm's wall time spent on the pinned core by anything but the arm (DECISIONS R15). `R2_TEST_MODE=1` is for tests only (see `DECISIONS.md` R1, T5.1).
