# Phase 1: tools (Tasks 1-7)

Read `README.md` first: Global Constraints, Protocol constants and Shared interfaces apply to every task.

Cargo commands in this phase run from `round2/` after `. round2/env.sh`, as
`msx_cargo r2-tool test -p r2_referee` (target dir `work/target/r2-tool`).

---

### Task 1: Clone, scaffolding, env, decision ledger

**Files:**
- Create: `round2/env.sh`, `round2/rust-toolchain.toml`, `round2/Cargo.toml`, `round2/referee/Cargo.toml`, `round2/referee/src/lib.rs`, `round2/DECISIONS.md`, `round2/results/ledger.jsonl` (empty), `round2/results/clone.json`, `round2/README.md`

**Interfaces:**
- Produces: `round2/env.sh` exporting `R2_ROOT=$MSX_ROOT/round2`, `R2_CLONE=$MSX_WORK/cqlite-r2`, `R2_WT=$MSX_WORK/cqlite-r2-wt`, `R2_BINS=$MSX_WORK/r2-bins`, `R2_LOCK=$MSX_WORK/r2.lock`, `R2_LEDGER=$R2_ROOT/results/ledger.jsonl`, `CQLITE_DATASETS_ROOT=/home/dev/projects/cqlite/test-data/datasets`, `R2_BASE=7f9abe3663254724439dbe71aff77bb93faa1c2d`; and function `r2_require_unlocked` (returns 1 with a message when `$R2_LOCK` is held, checked with `flock -n`).

- [ ] **Step 1:** `git clone --no-hardlinks /home/dev/projects/cqlite "$MSX_WORK/cqlite-r2"` then `git -C "$MSX_WORK/cqlite-r2" checkout -b r2/base 7f9abe3663254724439dbe71aff77bb93faa1c2d`.
- [ ] **Step 2: Verify.** `git -C work/cqlite-r2 rev-parse HEAD` prints `7f9abe3663254724439dbe71aff77bb93faa1c2d`; `git -C /home/dev/projects/cqlite status --porcelain` output is unchanged from before Step 1.
- [ ] **Step 3:** Write `round2/results/clone.json`: `{"cqlite_commit": "7f9a…", "cloned_at": <ISO UTC>, "cqlite_toolchain": <contents of the clone's rust-toolchain(.toml) or null>}`. If CQLite pins a toolchain other than 1.97.1, add a row to `DECISIONS.md` (D16) saying CQLite's pin is used for CQLite builds.
- [ ] **Step 4:** Write `env.sh` per Interfaces; workspace `Cargo.toml` (`members = ["referee"]`, `resolver = "2"`, release `opt-level = 3`, `debug = 1`); `referee/Cargo.toml` (package `r2_referee`, bin `r2` at `src/bin/r2.rs`, deps from README Tech Stack); empty `lib.rs`.
- [ ] **Step 5:** `DECISIONS.md`: copy spec section 11's table verbatim, header "Append rows; never edit a past row's decision — add a superseding row."
- [ ] **Step 6: Verify.** `. round2/env.sh && msx_cargo r2-tool build -p r2_referee` succeeds; `ls work/target/r2-tool` exists; `/home/dev/.cache-dev/cargo-target` mtime unchanged.
- [ ] **Step 7: Commit** `round2: scaffolding, clone record, decision ledger`.

---

### Task 2: `stats.rs`

**Files:** Create `round2/referee/src/stats.rs` (start from a copy of `experiment/harness/src/stats.rs`), tests inline in `#[cfg(test)]`.

**Interfaces:** Produces the `stats.rs` block in README "Shared interfaces". Drop round 1's `session_ratio` and `Sample` dependency; keep `median`, `percentile`, `bootstrap_ci`, `threshold`, `verdict` logic and constants unchanged.

- [ ] **Step 1: Write failing tests.**
  - `bootstrap_matches_round1`: build round 1's harness lib is not allowed (read only), so instead hard-code: for `ratios = [0.95, 0.97, 0.96, 0.98, 0.94, 0.96, 0.97, 0.95, 0.96, 0.99]`, compute with a byte-identical copy of round 1's `bootstrap_ci` kept in the test module (`round1_bootstrap_ci`) and assert the new `bootstrap_ci` returns exactly equal `point`, `lo`, `hi`.
  - `verdict_table`: with `t = 0.05`: `(0.90,0.88,0.94)→Won`, `(1.10,1.06,1.12)→Lost`, `(1.0,0.96,1.04)→Tied`, `(0.95,0.93,0.97)→Inconclusive`, `(0.95,0.949,1.0)→Inconclusive`.
  - `threshold_floor`: `threshold(0.01)==0.05`, `threshold(0.04)==0.08`.
  - `set_interval_geomean`: two cases with per-session ratios `{a:0.81,b:1.0}` ×10 sessions → `point` within 1e-12 of `0.9`.
  - `set_interval_missing_case_errors`: a session lacking case `b` → `Err` containing `"b"`.
  - `verdict_serializes_upper`: `serde_json::to_string(&Verdict::Won) == "\"WON\""`.
- [ ] **Step 2:** Run `msx_cargo r2-tool test -p r2_referee stats` → fails (not defined).
- [ ] **Step 3:** Implement per Interfaces.
- [ ] **Step 4:** Tests pass.
- [ ] **Step 5: Commit** `r2: stats (round 1 bootstrap, set intervals)`.

---

### Task 3: `criterion.rs`

**Files:** Create `round2/referee/src/criterion.rs`; fixtures `round2/referee/tests/fixtures/criterion_home/` (hand-written `benchmark.json` + `sample.json` pairs in Criterion 0.5.1's format; copy one real pair from a scratch `cargo bench` run of any small crate, or from `/home/dev/projects/cqlite/target/criterion` if present, then edit); test `round2/referee/tests/criterion.rs`.

**Interfaces:** Produces `CaseStat`, `read_home`. Per-iteration time per sample = `times[i] / iters[i]`; `median_ns` = median of those; `mad_ns` = median absolute deviation of those.

- [ ] **Step 1: Failing tests.**
  - `reads_two_cases`: fixture with `decode/type_int` (iters `[1,2,3]`, times `[10,22,30]`) → `median_ns == 10.0`, `samples == 3`.
  - `full_id_not_dirname`: fixture whose directory is `read/get_partition_big_bti_` but `benchmark.json` `full_id` is `read/get_partition_big (bti)` → key is the `full_id`.
  - `ignores_base_and_change_dirs`: only `new/` is read.
  - `empty_home_is_error`.
- [ ] **Step 2:** Run → fail. **Step 3:** Implement. **Step 4:** Pass.
- [ ] **Step 5: Commit** `r2: read Criterion homes by full_id`.

---

### Task 4: `host.rs` and `ledger.rs`

**Files:** Create `round2/referee/src/host.rs`, `round2/referee/src/ledger.rs`; tests inline using a `FakeProbe`.

**Interfaces:** Produces the `host.rs` and `ledger.rs` blocks in README. `ProcProbe` reads `/proc/loadavg`, `/proc/stat`, and scans `/proc/*/comm` for exactly `cargo` or `rustc`. `SessionMonitor` runs a 1 s sampling thread; involuntary switches come from `/proc/<pid>/status` `nonvoluntary_ctxt_switches` of each bench process, read just before it exits (the session runner passes pids in). Ledger ids are `R2-` + 6 digits, the next after the highest in the file; `append` takes an exclusive `flock` on `<ledger>.lock`, writes one line with `id`, `kind`, `at` (ISO UTC), `machsmith` (= `machsmith_state()`), then the body fields, and `fsync`s. `machsmith_state().dirty_paths` = `git status --porcelain` paths.

- [ ] **Step 1: Failing tests.**
  - `quiet_wait_needs_full_hold`: fake probe quiet for 9 simulated minutes, then a `cargo` pid at minute 9, then quiet → returns only after minute 19 (count `sleep` calls of 60 s: ≥ 19).
  - `quiet_wait_times_out`: always loaded → `Err` containing `"timeout"`.
  - `monitor_rejects_load`: load sample 3.1 → `Some(Rejection::Load(3.1))`; 2.9 → `None`.
  - `monitor_rejects_cargo`: a cargo pid in any sample → `CargoRustc`.
  - `monitor_rejects_sibling`: sibling util 0.25 → `SiblingUtil`.
  - `ctx_switch_rate`: 600 switches over 10 s → rejected; 400 → not.
  - `ledger_ids_increase`: two appends to a temp file → `R2-000001`, `R2-000002`; file has 2 lines; first line unchanged after second append (compare bytes).
- [ ] **Step 2:** Fail. **Step 3:** Implement. **Step 4:** Pass.
- [ ] **Step 5: Commit** `r2: host gate, session monitor, ledger`.

---

### Task 5: `session.rs`, `run.rs`, `r2` CLI

**Files:** Create `round2/referee/src/session.rs`, `round2/referee/src/run.rs`, `round2/referee/src/bin/r2.rs`; test `round2/referee/tests/run.rs`; fixture `round2/referee/tests/fixtures/fake_bench.sh`.

**Interfaces:**
- Consumes Tasks 2-4.
- Produces `Arm`, `RunSpec`, `RunKind`, `run`, `RunEnv { ledger: PathBuf, lock: PathBuf, scratch: PathBuf, probe: Box<dyn Probe>, quiet: bool }` (`quiet=false` only in tests: skips `quiet_wait`), and the CLI in README.

Behaviour (spec 6.2-6.7):
- Take `flock` on `$R2_LOCK` for the whole run. Run `quiet_wait` (hold 10 min, load 2.0). `choose_core`.
- Refuse before any session when `base.sha256 == cand.sha256` and kind ≠ `Null`.
- Refuse (status `refused`) a non-null run whose `--null` record is older than 7 days, is not `OK`, or was taken on a different base binary sha256 than `--base`.
- Per session `s` with seed `session_seed(s, replacements, nonce)` (round 1's formula): order the two arms by the seed; for each arm, run `taskset -c <core> perf stat -x, -e cycles:u,instructions:u -o <scratch>/perf.<arm> -- <bin> <criterion args>` with `CRITERION_HOME=<scratch>/s<s>/<arm>`, `CQLITE_DATASETS_ROOT`, and an env var `R2_PAD` of random length 0..4095 (seeded). Session timeout 1800 s.
- Session result: per case `cand.median_ns / base.median_ns` over the union of case ids; any id missing in either arm → `MissingCase`; non-zero exit → `ProcessFailed(stderr tail 2 KiB)`; monitor rejection as Task 4. A rejected session is replaced; the 4th rejection ends the run `NOISE_GATE_FAILED`.
- Verdicts: for `Confirm`, `Screen`, `Crosscheck`, `Proposal`: require `--null`; compute `b` with `nullband` on the same `targets` (and on the whole workload), `T = threshold(max(b_targets, b_workload))`; verdict on targets (if non-empty) and on the whole workload; per-case intervals; `REGRESSION` flag for any case with `lo > 1 + T`; `perf_gate_10pct` column = per-case point ratio > 1.10. For `Null`: intervals per case, per workload, and per-session per-case ratios stored so `nullband` can recompute any target set. For `Calibration`: verdict with `T` from the given null.
- Record: one ledger entry with everything in spec 6.7 plus `status` (`OK`, `NOISE_GATE_FAILED`, `aborted`, `refused`). On SIGINT/SIGTERM or panic: append `aborted` with completed sessions, release lock.
- `Screen` records carry `"provisional": true`.

- [ ] **Step 1: Write `fake_bench.sh`.** Reads `FAKE_FACTOR` (default 1.0), `FAKE_CASES` (comma list, default `g/a,g/b`), `FAKE_EXIT` (default 0); writes a Criterion-format `new/benchmark.json` + `sample.json` per case under `$CRITERION_HOME` with times = `1000 × FAKE_FACTOR × (1 + small seeded jitter)` per iter; exits `FAKE_EXIT`. Two copies of the script with different env wrappers serve as the arms.
- [ ] **Step 2: Failing tests** (`quiet=false`, fake probe always quiet):
  - `null_on_identical_is_tied`: Null, both arms same script → workload interval contains 1.0, band < 0.01.
  - `slow_arm_loses`: Calibration, cand `FAKE_FACTOR=1.2` → `LOST`.
  - `fast_arm_wins_on_targets`: Confirm with targets `["g/a"]`, cand `FAKE_FACTOR` 0.8 on `g/a` only (add a `FAKE_FACTOR_CASES` list to the script) → target verdict `WON`, whole-workload point within 0.01 of 0.894 (√0.8).
  - `missing_case_rejects` (Review Focus 1): cand `FAKE_CASES=g/a` → every session rejected → status `NOISE_GATE_FAILED`, rejection causes all `MissingCase("g/b")`.
  - `process_failure_rejects` (Review Focus 2): cand `FAKE_EXIT=3` → `ProcessFailed`, status `NOISE_GATE_FAILED` after 4 rejections.
  - `same_sha_refused` (Review Focus 4): Confirm with identical files → status `refused`, zero sessions run.
  - `abort_records` (Review Focus 5): send SIGTERM to the `r2` process after the first session (spawn the CLI binary; fake bench sleeps 1 s) → last ledger line has `status: "aborted"`, `sessions_completed ≥ 1`; `$R2_LOCK` not held afterwards (`flock -n` succeeds).
  - `stale_null_refused`: a null record with `at` 8 days ago → `refused`.
  - `regression_flagged`: Crosscheck with cand 1.3× on `g/b` → `g/b` flagged `REGRESSION`.
- [ ] **Step 3:** Fail. **Step 4:** Implement. **Step 5:** Pass.
- [ ] **Step 6: Commit** `r2: session runner, verdicts, CLI`.

---

### Task 6: `build.sh`, `test.sh`

**Files:** Create `round2/build.sh`, `round2/test.sh`.

**Interfaces:**
- `round2/build.sh <rev> <bench>`: `r2_require_unlocked || exit 1`; ensure worktree `$R2_WT/<rev-sha12>` (`git -C $R2_CLONE worktree add --detach`); `msx_cargo r2-bench-<sha12> bench -p cqlite-core --features cli-helpers,bench-internals --bench <bench> --no-run` inside it (use the feature set the bench's `required-features` needs; read it from `cqlite-core/Cargo.toml`); copy the newest `deps/<bench>-*` executable to `$R2_BINS/<sha12>/<bench>`; print `<path> <sha256>`. Same rev + bench twice → second call rebuilds nothing and prints the same sha (proves reproducibility the null run depends on).
- `round2/test.sh <rev>`: `r2_require_unlocked`; in the worktree, run CQLite's gate detached: `AGENT_GATE_SUMMARY_FILE=$MSX_WORK/r2-gate/<sha12>.summary nice -n 10 bash scripts/flow/gate-detached.sh` (read CQLite's `docs/development/gate-ops.md` for the exact invocation and terminal-token grammar before writing this), with `CARGO_TARGET_DIR=$MSX_WORK/target/r2-test`. If the summary says it cannot certify for a reason caused by being a clone (rebase/`origin/main`/tree-integrity), run the fallback: `nice -n 10 cargo test --workspace` and `RUSTFLAGS="-D warnings" nice -n 10 cargo clippy --workspace --all-targets --all-features`, same target dir. Append a ledger record `kind: "tests"` with rev, mode (`gate`|`fallback`), the summary block or the fallback's pass/fail counts and failing test names, and exit status.

- [ ] **Step 1:** Write `build.sh`. **Step 2: Verify:** `round2/build.sh $R2_BASE partition_lookup` twice → identical sha256 both times; binary runs `--list` and prints ≥ 1 case.
- [ ] **Step 3:** Write `test.sh`. Its full verification is Task 8. **Step 4: Verify** the refusal: hold `$R2_LOCK` with `flock $R2_LOCK sleep 30 &` and run both scripts → both exit 1 with the lock message.
- [ ] **Step 5: Commit** `round2: build and test wrappers`.

---

### Task 7: `profile.sh`, `attribute.py`

**Files:** Create `round2/profile.sh`, `round2/attribute.py` (start from a copy of `tools/ceiling.py`), `round2/test_attribute.py` (start from `tools/test_ceiling.py`).

**Interfaces:**
- `round2/profile.sh <bin> <workload> <tag>`: `r2_require_unlocked`; take `$R2_LOCK` (profiling is a measurement); `r2 quiet-wait`; for each case id from `<bin> --list`, `taskset -c <core> perf record -e cycles:u -F 4999 -o $MSX_WORK/r2-profile/<tag>/<workload>/<case>.data -- <bin> --bench --profile-time 20 '^<escaped case>$'`; then `perf script` to `.script` and save `.mmap` (the corrected version of round 1's misplaced lines, placed after the record). Record host snapshot at start into `<tag>/<workload>/host.json`. With `R2_PROFILE_CALLGRAPH=1`, use `--call-graph dwarf -F 999` instead and write under `<tag>-cg/`.
- `round2/attribute.py <dir> <bin> <out.json>`: round 1's buckets and libc naming, plus (a) **per-case** shares (one script file per case), (b) workload shares = sample-weighted over cases, (c) for `-cg` dirs, **inclusive** call-tree shares for named frames: allocator, `core::hash::sip`, `bcmp`/`memcmp`, `CqlType::parse_with_depth`, `Arc<str>` clone/drop, `Value::into_owned`, plus the top 30 inclusive CQLite functions. Output keys: `samples`, `per_case: {case: {bucket: share}}`, `workload: {bucket: share}`, `inclusive: {frame: share}` (cg only), `top_self: [...]`.

- [ ] **Step 1: Failing tests** in `test_attribute.py`: keep round 1's `CASES` classification test; add `test_per_case_shares_sum_to_100`, `test_workload_weighted_by_samples` (two cases, 100 and 300 samples, allocator 10% and 50% → workload 40%), `test_inclusive_counts_once_per_sample` (a sample with `malloc` twice in its chain counts once).
- [ ] **Step 2:** `python3 -m unittest round2/test_attribute.py` → fail. **Step 3:** Implement. **Step 4:** Pass.
- [ ] **Step 5: Verify on real data, short:** `R2_PROFILE_SHORT=1` (profile-time 2, one case) on `partition_lookup` produces a `.script` with > 1,000 samples and `attribute.py` output whose buckets sum to 100 ± 0.1.
- [ ] **Step 6: Commit** `round2: per-case profiler and attribution`.
