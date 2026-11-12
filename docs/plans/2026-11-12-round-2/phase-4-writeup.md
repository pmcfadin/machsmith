# Phase 4: write-up (Tasks 12-13)

### Task 12: Proposal sizing and write-up

- [ ] **Step 1: Proposal sizing** (only if proposals exist and every fix branch has finished Steps 8-9). For each proposal: branch `r2-proposal/<rule-id>`, make the API-changing fix, build, `r2 run --kind proposal --sessions 20 …`. Append the result to the proposal entry, labelled "proposal, not a fix". Skipped proposals say "not sized: out of time" in the write-up.
- [ ] **Step 2: Rule catalogue index** `docs/rules/README.md`: one row per rule and proposal: id, workload, predicted range, outcome, prediction held, paid off, link.
- [ ] **Step 3: Write-up** `docs/results/2026-11-round-2-cqlite-hotspots.md`, round 1's style and section order adapted:
  1. Answer (what got faster, by how much, on which operation, with ledger ids; or that nothing did).
  2. What was tested (commit, workloads, protocol summary, decisions with a link to `round2/DECISIONS.md`).
  3. Where CQLite's time goes now (HOTSPOTS.md summary, confirmation of round 1's hypotheses).
  4. Results per hot spot (rule, prediction, verdict table with `b`, `T`, ratio, interval, ids; cross-check table).
  5. What the winners did.
  6. What did not win and why (TIED, LOST, INCONCLUSIVE, unmeasurable, failed tests, not attempted, proposals).
  7. Limitations: one shared host; small cache-resident data; Criterion wall time primary; `bench` profile build (debuginfo on); correctness "tested by CQLite's suite only"; one model and budget; target sets chosen from the same profile that motivated the fix.
  8. What this suggests for the tool (rules that held, detection signals that worked).
  9. Reproducing this.
  - Every number cites a ledger id. No claim smaller than `T`. Never "optimal". No "CQLite is X% faster" beyond the operation measured.
- [ ] **Step 4: Verify traceability:** a script (`round2/check_citations.py`, stdlib only) extracts every `R2-\d{6}` in the write-up and rule entries and confirms each exists in the ledger; every ratio in the write-up's verdict tables matches the cited record to the printed precision. Run it; it must print `OK`.
- [ ] **Step 5: Commit** `docs: round 2 write-up and rule catalogue`.

### Task 13: Final review and owner message

- [ ] **Step 1:** Final review by a fresh reviewer on the most capable model (`claude-opus-5-5`) over the write-up, the rule catalogue, `DECISIONS.md` and the ledger, against the spec and the brief: traceability, honesty rules, stop-and-ask compliance, round 1 artifacts untouched (`sha256sum work/target/tool/release/msx` starts `9007b7ac`; `git diff <round-1 commit> -- experiment tools docs/results/2026-11-llm-asm-experiment.md` empty; `git -C work/cqlite status` clean).
- [ ] **Step 2:** Fix findings; commit `docs: round 2 write-up fixes (final review)`.
- [ ] **Step 3:** Tell the owner in a few sentences: what got faster and by how much with ledger ids, which rules held, what is left (unattempted spots, proposals, unmeasurable branches), and that CQLite branches await review in `work/cqlite-r2`. Ask before pushing machsmith.
