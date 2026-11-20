# Round 2 Phase 2 close-out Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Finish Phase 2 (Tasks 9 and 10 of `phase-2-baseline.md`) under the owner decisions of 2026-11-16/17: close Task 9, pre-register the new rules, take decode profile C and a read branch-stack profile, run the three nulls, and calibrate the ruler.

**Architecture:** The controller (the session running this plan) owns every measurement: it launches long runs as background jobs, appends ledger records and commits results. Subagents do the two pieces of new code (Task 4: branch-stack profiling and caller attribution; Task 8: the calibration branches in the CQLite clone), each followed by a fresh reviewer. Measurement tasks have no reviewer subagent; their check is the ledger record and the verification step.

**Tech Stack:** bash, `perf` 7.0.14 (AMD Zen 5, LBRv2), Python 3 stdlib, Rust 1.97.1 (`r2`), Criterion 0.5.1.

**Spec:** `docs/specs/2026-11-12-round-2-design.md`; parent plan `docs/plans/2026-11-12-round-2/README.md` and `phase-2-baseline.md`. Hot-spot evidence: `round2/results/profile/HOTSPOTS.md`.

## Global Constraints

Everything in the parent plan's "Global Constraints" holds. In particular:

- Never modify `/home/dev/projects/cqlite`. CQLite work happens in `work/cqlite-r2`, branches cut from `7f9abe3663254724439dbe71aff77bb93faa1c2d`.
- `CQLITE_DATASETS_ROOT=/home/dev/projects/cqlite/test-data/datasets` for every bench, profile and run.
- Source `round2/env.sh`; build only with `msx_cargo <name>`; never build or test while `work/r2.lock` is held (`r2_require_unlocked`).
- `round2/results/ledger.jsonl` and `round2/DECISIONS.md` are append-only. New DECISIONS rows go at the end of the table.
- No pushes, merges or PRs. Commit to local `main` at the end of every task.
- Stop-and-ask points (spec section 10) plus: the noise-gate soft cap (R21 below); a tool call refused by the auto-mode safety check twice in a row (report, do not loop).

Owner decisions this plan implements (settled 2026-11-16/17, recorded as DECISIONS R20-R24 in Task 2):

1. The 3-try null cap counts only completed nulls with `b > 0.05`. R2-000005..10 (NOISE_GATE_FAILED) count as zero tries.
2. Noise-gate soft cap: at most 3 further `NOISE_GATE_FAILED` nulls per workload, counted from the first null after R21's commit, each started ≥ 2 h after the previous attempt on that workload. Reaching it: stop and ask (not "ruler broken").
3. H2 gets a third decode profile C. H2 is confirmed iff C's H2 share is ≥ 3% and within 2 pp of A's (64.31) or B's (61.12). If C fails, H2 is skipped and not revisited.
4. H3/H5 get a branch-stack (LBRv2) profile of the read binary, structural evidence only; if branch sampling does not work in this container, both are skipped. No frame-pointer rebuild.
5. H1: internal fix (private fast-hash map in `MaterializedIndex`) counted toward the 4 fixes, plus a proposal for a hasher change on `IndexData::key_lookup`. (Implemented in Phase 3; only recorded here.)

Run autonomously: commit after each task, stop only at the stop-and-ask points.

## Review Focus

- `perf record -j` refused or silently producing samples with no branch records (container, `perf_event_paranoid = 2`): must be detected as "infeasible", not attributed as "no caller found". Pinned by Task 3's gate and Task 4's `test_empty_brstack_counts_as_no_branch_data`.
- Branch records whose `from` is inside glibc (malloc calling its own internals): must not be credited as the caller; the caller is the most recent call *from the bench binary*. Pinned by `test_skips_libc_internal_calls`.
- `R2_PROFILE_BRANCH=1` together with `R2_PROFILE_CALLGRAPH=1`: ambiguous output dir and perf args; must be refused. Pinned by Task 4 Step 5.
- A null attempt after the soft cap: the controller must count per workload, including attempts killed with no record (count them as attempts, record them in `attempts-null.md`). Pinned by Task 7's counting table.
- Pre-registration order: the H2 test (R22) must be committed before profile C starts. Task 5's first step checks `git log` for R22's commit.

---

### Task 1: Close Task 9 (controller)

**Files:**
- Create: `round2/profile_record.py` (from the scratch generator `mkbody.py`, generalised)
- Modify: `round2/results/profile/HOTSPOTS.md` (fill `<!-- CASE-SAMPLES -->`; replace "Decisions for the owner" with the settled outcomes)
- Modify: `round2/results/ledger.jsonl` (append only, via `r2 ledger-append`)

**Interfaces:**
- Produces: `python3 round2/profile_record.py <tag> <out.json> [workload ...]` writes a `kind: profile` body. Default workloads are the subdirectories of `work/r2-profile/<tag>/` that have a `host.json`. It exits non-zero if a binary's sha256 differs from `round2/results/profile/binaries.json`. For a tag whose `host.json` has `"branch": true`, the attribution file is the callers JSON (Task 4): it records that file's `samples`, `bucket_samples` and `brstack_empty_pct`, and no `case_samples` or chain stats.

- [ ] **Step 1:** Copy the scratch `mkbody.py` (`/tmp/claude-1001/-home-dev-projects-machsmith/abc55852-b48d-43f5-ad75-8d70fb5e9d9c/scratchpad/mkbody.py`; if it is gone, rewrite it from the interface above) to `round2/profile_record.py` with optional workload args and the `note` keyed by tag (`A`, `B`, `A-cg` as now; `C`: "third decode profile for H2 (DECISIONS R22)"; `A-br`: "read branch-stack profile for H3/H5, structural only (DECISIONS R23)").
- [ ] **Step 2:** Generate and append, in order: `for t in A B A-cg; do python3 round2/profile_record.py $t /tmp/…/body-$t.json && R2_LEDGER=round2/results/ledger.jsonl work/target/r2-tool/debug/r2 ledger-append --kind profile --body-file /tmp/…/body-$t.json; done`. Expected: prints `R2-000011`, `R2-000012`, `R2-000013`. If the ids differ, fix every id in HOTSPOTS.md to match.
- [ ] **Step 3:** Replace `<!-- CASE-SAMPLES -->` with one table per workload: `| case | A samples | B samples |`, from `case_samples` in `round2/results/profile/{A,B}/<w>.json`.
- [ ] **Step 4:** Replace HOTSPOTS.md's "Decisions for the owner" section with "Owner decisions (2026-11-17)", listing decisions 2-5 from Global Constraints with their DECISIONS row ids (R22-R24).
- [ ] **Step 5: Verify:** `tail -3 round2/results/ledger.jsonl | jq -r '[.id,.kind,.tag]|@tsv'` shows the three records; `grep -c CASE-SAMPLES round2/results/profile/HOTSPOTS.md` prints 0.
- [ ] **Step 6: Commit** `results: round 2 profile and hot-spot ranking` (HOTSPOTS.md, ledger.jsonl, profile_record.py).

### Task 2: Pre-register owner decisions (controller)

**Files:**
- Modify: `round2/DECISIONS.md` (append R20-R24)
- Create: `round2/results/attempts-null.md`

- [ ] **Step 1:** Append rows R20-R24, one per owner decision 1-5 above, in the existing `| # | decision | why | cost if wrong |` format. R22 must list H2's six physical functions verbatim from HOTSPOTS.md ("H2" frames), the test (C ≥ 3% and |C − 64.31| < 2 or |C − 61.12| < 2), the C target rule (cases ≥ 3% in A, B and C), and "C is taken by `profile.sh <decode bin> decode C` under the normal quiet wait". R23 states the branch command (`-j any_call,u`), that its output is structural only, and the skip rule.
- [ ] **Step 2:** Create `round2/results/attempts-null.md` with a table `| workload | attempt | started (UTC) | ledger id | status | b | counts toward |`, pre-filled with R2-000005..10 marked "pre-R20, counts toward nothing", and the 2026-11-14T01:08Z read attempt (killed, no record) marked the same way.
- [ ] **Step 3: Commit** `round2: owner decisions R20-R24 (pre-registered)`. Record the commit hash; Task 5 checks it.

### Task 3: Branch-sampling feasibility gate (controller, ~1 min)

- [ ] **Step 1:** Run, from `round2/` with `env.sh` sourced: `CQLITE_DATASETS_ROOT=… taskset -c 2 perf record -e cycles:u -F 999 -j any_call,u -o $R2_SCRATCH/br.data -- work/r2-bins/7f9abe366325/read --bench --profile-time 2 read/type_heavy`.
- [ ] **Step 2:** `perf script -i $R2_SCRATCH/br.data -F ip,sym,dso,brstack | head -50`.
- [ ] **Step 3: Gate.** Feasible iff `perf record` exited 0 and ≥ 90% of sample lines carry at least one `0x…/0x…/` brstack entry. Record the result (pass/fail, counts, the exact perf error if any) as a new DECISIONS row R25 ("observation, no code") and commit `round2: branch-sampling feasibility (R25)`. If infeasible: skip Task 4 and the branch half of Tasks 5-6; HOTSPOTS gets "H3/H5 skipped: branch sampling infeasible (R25)".

### Task 4: Branch-stack profiling and caller attribution (subagent + reviewer)

Only if Task 3 passed.

**Files:**
- Modify: `round2/profile.sh` (new `R2_PROFILE_BRANCH=1` mode)
- Create: `round2/callers.py`, `round2/test_callers.py`

**Interfaces:**
- Consumes: `attribute.Context(binary)`, `attribute.classify(function, dso)`, `attribute.resolve_inline`, the `.mmap` files `profile.sh` writes, and `attribute.frame_addr` for runtime→file address mapping.
- Produces:
  - `profile.sh` with `R2_PROFILE_BRANCH=1`: output under `work/r2-profile/<tag>-br/<workload>/`, `perf record -e cycles:u -F 999 -j any_call,u`, `perf script -F ip,sym,symoff,dso,brstack` into `<slug>.script`, `host.json` gains `"branch": true`.
  - `parse_brstack(field: str) -> list[tuple[int, int]]`: `(from, to)` pairs, most recent first, from perf's `0xFROM/0xTO/P/-/-/0` tokens.
  - `caller_site(ctx, leaf_dso: str, brstack: list[tuple[int, int]]) -> str | None`: the most recent entry whose `from` lies in the bench binary (not libc, not PLT), resolved through inlines to the innermost `cqlite_core` frame (`function` + `file:line`), else the innermost frame of any name; `None` if no entry qualifies.
  - CLI `callers.py <dir> <bin> <out.json>`: for samples whose leaf bucket is `allocator` or `memcpy`, a JSON `{ "samples": int, "bucket_samples": {bucket: int}, "brstack_empty_pct": float, "no_caller_pct": {bucket: float}, "sites": {bucket: [{"site": str, "share_of_bucket": float, "share_of_workload": float}] (top 25)}, "per_case_sites": {case: {bucket: top 10}} }`.

- [ ] **Step 1: Write failing tests** in `round2/test_callers.py` (same style as `test_attribute.py`):
  - `test_parse_brstack_order`: `parse_brstack("0x10/0x20/P/-/-/0 0x30/0x40/P/-/-/0")` gives `[(0x10, 0x20), (0x30, 0x40)]`.
  - `test_skips_libc_internal_calls`: with a stub context mapping 0x1000-0x1fff to the bench binary and 0x9000-0x9fff to libc, `caller_site` on `[(0x9010, 0x9100), (0x1010, 0x8000)]` returns the site for 0x1010.
  - `test_empty_brstack_counts_as_no_branch_data`: a sample with no brstack entries goes into `brstack_empty_pct`, not into `no_caller_pct`.
  - `test_prefers_cqlite_frame_in_inline_chain`: an inline chain `[alloc::raw_vec::…, cqlite_core::x::y]` yields `cqlite_core::x::y`.
- [ ] **Step 2:** `cd round2 && python3 -m unittest test_callers -v`. Expected: FAIL (module missing).
- [ ] **Step 3:** Implement `round2/callers.py` per the interfaces.
- [ ] **Step 4:** Run the tests again (`test_callers` and `test_attribute`). Expected: all PASS.
- [ ] **Step 5:** Add the `R2_PROFILE_BRANCH` mode to `profile.sh`. It refuses (exit 2, message names both vars) when `R2_PROFILE_CALLGRAPH=1` is also set. Verify: `R2_TEST_MODE=1 R2_PROFILE_SHORT=1 R2_PROFILE_BRANCH=1 round2/profile.sh work/r2-bins/7f9abe366325/read read t4br` exits 0 and `work/r2-profile/t4br-br/read/*.script` lines carry brstack tokens. Then `python3 round2/callers.py work/r2-profile/t4br-br/read work/r2-bins/7f9abe366325/read /tmp/…/t4.json` prints a JSON whose `bucket_samples.allocator > 0`. Also verify the refusal.
- [ ] **Step 6: Commit** `round2: branch-stack profiling and caller attribution`.

### Task 5: Measurement window 1: decode C and read branch stacks (controller, background)

- [ ] **Step 1:** `git log --oneline` contains Task 2's commit, and it is older than now. Otherwise stop.
- [ ] **Step 2:** One background job, logged to `work/r2-logs/p2b-window1.log`: `round2/profile.sh work/r2-bins/7f9abe366325/decode decode C`, then (if Task 3 passed) `R2_PROFILE_BRANCH=1 round2/profile.sh work/r2-bins/7f9abe366325/read read A`. Each waits for a quiet host itself (`r2 quiet-wait`, 12 h timeout). Do not pin C's core: profile.sh chooses it, as for A.
- [ ] **Step 3:** When it exits: `python3 round2/attribute.py work/r2-profile/C/decode work/r2-bins/7f9abe366325/decode round2/results/profile/C/decode.json`; if branch ran, `python3 round2/callers.py work/r2-profile/A-br/read work/r2-bins/7f9abe366325/read round2/results/profile/A-br/read.json`.
- [ ] **Step 4:** Append `kind: profile` records for `C` and (if run) `A-br` with `profile_record.py`. Note the ids.
- [ ] **Step 5: Commit** `results: decode profile C and read branch stacks` (results JSONs, ledger).

### Task 6: Evaluate H2, attribute H3/H5 (controller)

- [ ] **Step 1:** Compute C's H2 share with the same grouping as `groups.py` in the scratch dir (physical-function membership from R22). Apply R22's test. Compute the C-qualified target set (cases ≥ 3% in A, B and C).
- [ ] **Step 2:** Append a section "Profile C (H2) and branch stacks (H3/H5)" to HOTSPOTS.md: C's share, the test result with arithmetic, the target set (or "H2 skipped per R22"); for H3/H5, the top 10 call sites per bucket with `share_of_bucket`, `brstack_empty_pct` and `no_caller_pct`, and whether a site covers ≥ 25% of its bucket (a candidate for a rule) or the cost is spread out (no rule). Do not edit the earlier ranking table; this section supersedes it where they differ.
- [ ] **Step 3: Commit** `results: H2 profile C verdict, H3/H5 call sites`.

### Task 7: Null runs (controller, background)

**Files:** `round2/results/attempts-null.md` (one row per attempt), ledger via `r2 run`.

- [ ] **Step 1:** For each workload in order read, decode, partition_lookup: the candidate is `work/r2-bins/7f9abe366325/<w>.copy`, as used by R2-000005..10 (it exists; check that its sha256 equals the base's before the first run). Launch `r2 run --kind null --workload <w> --base <bin> --cand <bin>.copy --sessions 20` in a background job logged to `work/r2-logs/p2b-null-<w>-<n>.log`.
- [ ] **Step 2:** After each run, add an `attempts-null.md` row. Then:
  - `OK`: `r2 nullband --null <id> --workload <w>`. Record `b` and `T`. If `b ≤ 0.05`, that workload is done. If `b > 0.05`, that is a counted try (R20). Retry ≥ 2 h later. Three counted tries over: **stop and report: ruler broken.**
  - `NOISE_GATE_FAILED`: count it toward R21's cap. Retry ≥ 2 h after this attempt's start. The third such failure for a workload: **stop and ask.**
  - `aborted` or killed with no record: add a row and count it toward R21. Then retry.
- [ ] **Step 3:** Commit after each workload finishes (`results: null <w> (R2-…)`), including `attempts-null.md` and the ledger.

### Task 8: Ruler calibration (subagent for CQLite code + reviewer; controller runs)

Follows `phase-2-baseline.md` Task 10 Steps 3-7 exactly. Additions:

- [ ] **Step 1 (subagent):** Branch `r2/cal-slow` from `7f9abe36` in `work/cqlite-r2` (worktree under `$R2_WT`). Add a `std::hint::black_box` spin loop in `V5CompressedLegacyParser::parse_row_data_with_offset_impl` (`row_decoder/row_data.rs`), the top CQLite function every read case reaches (self share 1.4% `type_heavy` to 25.5% `clustering_slice` in profile A, so the planted slowdown is uneven across cases; the 1.15 target is on the whole-workload ratio). The loop count is a `const CAL_SPIN: u32` at the top of that file, so sizing changes one line. Commit on the branch.
- [ ] **Step 2 (subagent):** Branch `r2/cal-noop` from `7f9abe36`. Rename one private function on the read path (e.g. a `fn` in `row_decoder/row_framing.rs` that is not `pub`/`pub(crate)`) and its call sites; no other change. Commit.
- [ ] **Step 3 (reviewer subagent):** Check both diffs: cal-slow touches only the spin and its const; cal-noop is a pure rename of a private item; neither touches tests or `pub` items.
- [ ] **Step 4 (controller):** `round2/build.sh <sha> read` for both branches. For cal-noop, confirm its sha256 differs from base's `773f4a2e…`.
- [ ] **Step 5 (controller):** Size `CAL_SPIN` with 5-session screening runs (`r2 run --kind screen --workload read … --null <read null id> --sessions 5`) until the whole-workload ratio is in 1.12-1.18. Each resize is one commit on `r2/cal-slow` and one row in `round2/results/ruler-check.md` (labelled provisional).
- [ ] **Step 6 (controller):** Calibration runs (`--kind calibration`, 20 sessions) for cal-slow (expected `LOST`) and cal-noop (expected `TIED`), under the noise-gate rules of Task 7. Either expectation failing: **stop and report**, change nothing.
- [ ] **Step 7:** Write `round2/results/ruler-check.md` per Task 10 Step 6. **Commit** `results: null band and ruler calibration`. Then report to the owner: Phase 2 done, H2 and H3/H5 outcomes, Phase 3 candidate list in rank order.
