# Phase 2: baseline (Tasks 8-10)

These tasks run the tools; they write results, not code. Each long run happens in a quiet-host window
(`r2 quiet-wait` gates it) and runs in the background; the controller waits for it rather than polling.
Never build or test while one is running.

---

### Task 8: Base commit test suite

- [ ] **Step 1:** `round2/test.sh $R2_BASE` (background; it can take hours).
- [ ] **Step 2: Verify:** a `kind: "tests"` ledger record exists for `7f9abe36` with mode and outcome.
- [ ] **Step 3:** Write `round2/results/base-tests.md`: mode, pass/fail, and the list of failing tests (these are "pre-existing" for every branch). If mode is `fallback`, add a `DECISIONS.md` row (deviation, why the gate could not certify).
- [ ] **Step 4: Stop-and-ask** if failures make the suite unusable as a referee (for example most dataset tests fail). Otherwise continue.
- [ ] **Step 5: Commit** `results: base test suite`.

---

### Task 9: Profiles and hot-spot ranking

- [ ] **Step 1:** Build base binaries: `round2/build.sh $R2_BASE read`, `… decode`, `… partition_lookup`. Record paths and sha256s in `round2/results/profile/binaries.json`.
- [ ] **Step 2:** Profile A: `round2/profile.sh <bin> <w> A` for the three workloads (flat, 20 s per case).
- [ ] **Step 3:** Profile B: same, at least 4 hours after A started, tag `B`.
- [ ] **Step 4:** Call-graph run: `R2_PROFILE_CALLGRAPH=1 round2/profile.sh <bin> <w> A` for the three workloads.
- [ ] **Step 5:** `attribute.py` on A, B and A-cg into `round2/results/profile/{A,B,A-cg}/<workload>.json`.
- [ ] **Step 6:** Write `round2/results/profile/HOTSPOTS.md`:
  - duration and samples per workload and per case, for A and B;
  - ranked table of hot spots: name, workload, inclusive share (A-cg), self share A and B, "confirmed" per spec section 5 (≥ 3% in both, differ < 2 pp);
  - per hot spot, the cases where its share is ≥ 3% (the candidate target set);
  - each round 1 hypothesis (README of the brief: decode type parsing, SipHash, bcmp, allocator, memcpy, `Arc<str>`, `into_owned`) marked confirmed / not confirmed / changed, with numbers;
  - a public-API note per hot spot: does the obvious fix touch a `pub` item? (check with `grep` of the item's visibility and the crate's `pub use` paths).
- [ ] **Step 7:** Append ledger records `kind: "profile"` for A, B and A-cg (paths, sample counts, host snapshots).
- [ ] **Step 8: Commit** `results: round 2 profile and hot-spot ranking`.

---

### Task 10: Null run and ruler calibration

**Files:** branches in `work/cqlite-r2`: `r2/cal-slow`, `r2/cal-noop`; `round2/results/ruler-check.md`.

- [ ] **Step 1: Null.** For each workload: copy the base binary to `$R2_BINS/<sha12>/<bench>.copy`, then `r2 run --kind null --workload <w> --base <bin> --cand <bin>.copy --sessions 20`. Note: `run` refuses equal shas for non-null kinds only.
- [ ] **Step 2:** For each workload, `r2 nullband --null <id> --workload <w>` (whole workload). If any `b > 0.05`: retry that workload's null at a different time (≥ 2 h later), up to 3 tries total. Three tries over → **stop and report: ruler broken.**
- [ ] **Step 3: Planted slowdown.** Branch `r2/cal-slow` from base. Add a `std::hint::black_box` spin loop on a path every `read` case reaches (pick from HOTSPOTS.md's top `read` function), sized by a 5-session screening run so the whole-workload ratio is about 1.15. Build; `r2 run --kind calibration --workload read --base <base read> --cand <slow read> --null <read null id> --sessions 20`. Expected: `LOST`.
- [ ] **Step 4: Planted no-op.** Branch `r2/cal-noop`: rename one private function on the `read` path. Its mangled symbol changes, so the binary's sha256 differs, but behaviour and code do not. Confirm sha differs. `r2 run --kind calibration … --sessions 20`. Expected: `TIED`.
- [ ] **Step 5:** If either expectation fails → **stop and report.** Do not change the protocol.
- [ ] **Step 6:** Write `round2/results/ruler-check.md`: per workload, `b`, `T`, null intervals and ledger ids; calibration verdicts, ratios, intervals and ids; host state per run; any noise-gate failures.
- [ ] **Step 7: Commit** `results: null band and ruler calibration`.
