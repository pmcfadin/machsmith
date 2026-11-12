# Phase 3: hot spots (Task 11, repeated)

Run Task 11 once per hot spot in `HOTSPOTS.md` rank order. Stop after **4 fixes**, or when the next
confirmed hot spot's predicted gain (Step 1) cannot beat `T` (its predicted ratio range lies entirely
above `1 − T`). A hot spot whose only fix needs a public-API change becomes a proposal (Step 1b) and
does not count toward the 4.

The controller does Steps 1, 6-10. A fresh agent from `hotspot-agent-prompt.md` does Steps 2-5.
A fresh reviewer checks the diff after Step 5 (spec compliance: internal-only, one hot spot, no test
edits that weaken assertions).

---

### Task 11.k: Hot spot `<rule-id>`

**Files:**
- Create: `docs/rules/<rule-id>.md` (before any code), `round2/results/targets/<rule-id>.json`
- CQLite: branch `r2/<rule-id>` in `work/cqlite-r2`, worktree `$R2_WT/<rule-id>`
- Modify: `round2/results/attempts.md`, `round2/DECISIONS.md` (if any decision arises)

`<rule-id>` is short kebab-case naming the pattern, e.g. `reparse-type-string-per-value`, `default-siphash-on-hot-map`, `per-cell-arc-str-clone`.

- [ ] **Step 1: Rule entry, before code.** Write `docs/rules/<rule-id>.md` with these headings:
  `Pattern`, `Detection` (source / MIR / machine-code signal a linter would use), `Evidence` (profile ledger ids, inclusive and self shares, per-case shares), `Target cases` (from HOTSPOTS.md, share ≥ 3%; also written as a JSON list of Criterion `full_id`s to `round2/results/targets/<rule-id>.json`), `Predicted gain` (ratio range on the target set with the arithmetic: share `s` of target time, fraction `f` expected to be removed, ratio ≈ `1 − s·f`, range from the low and high `f`), `Public API` (does the fix touch a `pub` item; route chosen), and an empty `Outcome` section.
  - Compute this rule's `T` now: `r2 nullband --null <workload null id> --workload <w> --targets round2/results/targets/<rule-id>.json`. Write `b` and `T` into the entry.
  - If the predicted range lies entirely above `1 − T`, mark the entry "not attempted: predicted gain below T" and move to the next hot spot.
  - **Commit** `rules: <rule-id> (pre-registered)` before Step 2. The commit hash is the proof of order.
- [ ] **Step 1b (proposal path):** if every fix route touches a `pub` item, move the entry to `docs/rules/proposals/<rule-id>.md`, add a `Proposed change` section (the API change, who is affected, migration), commit, and skip to the next hot spot. Sizing happens in Task 12.
- [ ] **Step 2 (agent):** create branch and worktree from `$R2_BASE`; implement the fix (attempt 1).
- [ ] **Step 3 (agent):** `round2/build.sh <branch sha> <bench>`; in a quiet window run `r2 run --kind screen --workload <w> --base <base bin> --cand <cand bin> --targets <targets.json> --null <null id>` (5 sessions). Append a row to `round2/results/attempts.md`: attempt n, branch sha, ledger id, target point ratio, labelled provisional.
- [ ] **Step 4 (agent):** if attempt 1 screens worse than the predicted range's upper bound, one more attempt (attempt 2) and screen it. No third attempt. The agent picks which attempt goes forward (its branch head must be that attempt).
- [ ] **Step 5 (agent):** `RUSTFLAGS="-D warnings" cargo clippy -p cqlite-core --all-targets` and `cargo fmt --check` in the worktree (target `work/target/r2-test`, `nice -n 10`, never in a measurement window). Commit on the branch.
- [ ] **Step 6: Review.** Fresh reviewer on the branch diff: internal only (no `pub` signature, field, or exported type changes; `cargo public-api` is not installed, so check `git diff` for `pub` lines and trait impls), no format or wire change, no weakened tests, change matches the rule's pattern.
- [ ] **Step 7: Tests.** `round2/test.sh <branch sha>`. Any failure not in `base-tests.md` → no confirmation; record the outcome "failed tests" in the rule entry; next hot spot.
- [ ] **Step 8: Confirm.** `r2 run --kind confirm --workload <w> --base <base bin> --cand <cand bin> --targets <targets.json> --null <null id> --rule <rule-id>` (20 sessions). `NOISE_GATE_FAILED` → retry in a later quiet window, at most 3 tries total; after 3, outcome "unmeasurable on this host on this date". Never raise the cap.
- [ ] **Step 9: Cross-check.** Build the candidate's other two bench binaries and run `r2 run --kind crosscheck --sessions 10` for each other workload against base, with `--null` for that workload. Same retry rule.
- [ ] **Step 10: Outcome.** Append (never edit above) the `Outcome` section of the rule entry: verdict on targets and on the whole workload, `b`, `T`, ratio, 95% interval, ledger ids, REGRESSION flags from the cross-check, `perf-gate` 10% column, "prediction held: yes/no" (point estimate inside the predicted range), "paid off: yes/no" (WON with no REGRESSION). Commit `rules: <rule-id> outcome`.
