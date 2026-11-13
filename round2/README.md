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
