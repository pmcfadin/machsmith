# Hot-spot agent prompt (template)

The controller fills every `{{placeholder}}` and sends the text below the line, and nothing else, to a fresh agent.

| Placeholder | Value |
|---|---|
| `{{RULE_ID}}` | the rule id |
| `{{RULE_PATH}}` | `docs/rules/{{RULE_ID}}.md` (committed, pre-registered) |
| `{{WORKLOAD}}` | `read`, `decode` or `partition_lookup` |
| `{{BASE_BIN}}` | path of the base bench binary |
| `{{TARGETS}}` | `round2/results/targets/{{RULE_ID}}.json` |
| `{{NULL_ID}}` | ledger id of the workload's null run |

---

You are making one CQLite benchmark faster with one source-level Rust change. A strict referee will judge it afterwards; an honest no-win is a fine result, a broken or out-of-scope win is not.

## Read first

1. `/home/dev/projects/machsmith/{{RULE_PATH}}`: the pattern, the evidence that it is hot, the target cases and the predicted gain. Your change fixes this pattern and nothing else.
2. `/home/dev/projects/machsmith/round2/results/profile/HOTSPOTS.md`: the profile.
3. `/home/dev/projects/machsmith/docs/plans/2026-11-12-round-2/phase-3-hotspots.md`, Steps 2-5: your steps.
4. CQLite's `CLAUDE.md` and `AGENTS.md` in the worktree: its coding rules (file size, no heuristics, real data only).

## Rules

1. **Work only in** `/home/dev/projects/machsmith/work/cqlite-r2-wt/{{RULE_ID}}` on branch `r2/{{RULE_ID}}`, created from `7f9abe3663254724439dbe71aff77bb93faa1c2d`. Never touch `/home/dev/projects/cqlite`, `work/cqlite`, or machsmith files other than appending to `round2/results/attempts.md`.
2. **Internal only.** Do not add, remove or change any `pub` item's signature, any `pub` field or its type, any exported type, any trait impl on a public type, the on-disk format or wire behaviour. If the fix cannot be done without that, stop and say so; do not make it.
3. **Do not weaken tests.** You may add tests. You may not delete or loosen an assertion.
4. **Builds:** `. /home/dev/projects/machsmith/round2/env.sh`, then only `round2/build.sh` for bench binaries and `msx_cargo r2-test …` (under `nice -n 10`) for clippy and targeted tests. Never the shared `CARGO_TARGET_DIR`. Never build while `$R2_LOCK` is held (the scripts refuse; do not work around them).
5. **Measuring:** only `r2 run --kind screen --workload {{WORKLOAD}} --base {{BASE_BIN}} --cand <your bin> --targets {{TARGETS}} --null {{NULL_ID}}`. At most two attempts, each screened once. Screening numbers are provisional and are not the verdict.
6. **Do not change** the rule entry, the target list, the referee, or any protocol setting.
7. Commit your chosen attempt on the branch with a message naming the rule id.

## Report back

Branch head sha; what you changed and why it addresses the pattern; screening ledger ids and target point ratios per attempt; which attempt you chose; anything you suspect could affect correctness; anything you were unsure was internal.
