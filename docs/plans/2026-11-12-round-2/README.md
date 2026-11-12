# Round 2: refereed speed-ups on CQLite's hot spots — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Profile current CQLite, then for up to four hot spots write a linter rule first, make one internal Rust fix per branch, and referee each fix end to end on CQLite's own Criterion benchmarks with round 1's null-band / noise-gate / bootstrap protocol.

**Architecture:** A new top-level `round2/` directory in machsmith. It holds a small Rust workspace with one binary, `r2`, the referee, which runs whole Criterion bench binaries in paired sessions, gates on host noise, bootstraps over sessions and appends to an append-only ledger. It also holds shell and Python tools for builds and profiling, and a results tree. CQLite work happens in a clone at `work/cqlite-r2` with one `git worktree` per branch. Phase 1 builds the tools with tests. Phase 2 measures the baseline: test suite, profile, null run, ruler calibration. Phase 3 loops over hot spots. Phase 4 writes up.

**Tech Stack:** Rust 1.97.1 (`serde`, `serde_json`, `sha2`, `rand_chacha 0.3`, `rand_core 0.6`, `libc`); Criterion 0.5.1 (CQLite's, unchanged); `perf`; `taskset`; Python 3 (stdlib only) for attribution.

**Spec:** `docs/specs/2026-11-12-round-2-design.md`. Read it first; this plan argues from it and does not repeat its reasoning. The source brief is `docs/prompts/2026-11-12-round-2-cqlite-hotspots.md`.

## Documents

| File | Contents |
|---|---|
| `README.md` (this file) | Constraints, file map, shared interfaces, protocol constants, task index |
| `phase-1-tools.md` | Tasks 1-7: clone and scaffolding, referee library and CLI, build and profile tools |
| `phase-2-baseline.md` | Tasks 8-10: base test suite, profile and hot-spot ranking, null run and ruler calibration |
| `phase-3-hotspots.md` | Task 11 (repeated per hot spot): rule, fix, gate, confirm, cross-check |
| `phase-4-writeup.md` | Tasks 12-13: write-up, rule catalogue, final review, owner message |
| `hotspot-agent-prompt.md` | Prompt template for the fresh agent that writes each fix |

Do the tasks in order. Phase 1's tasks depend on each other's interfaces (section "Shared interfaces").

## Global Constraints

- **Never modify `/home/dev/projects/cqlite`.** No edits, no builds inside it, no `git worktree add` from it. Reading files and `test-data/` is fine.
- **CQLite commit:** `7f9abe3663254724439dbe71aff77bb93faa1c2d`. The clone `work/cqlite-r2` is created at exactly this commit; every branch is cut from it. CQLite's HEAD moving is not a reason to rebase.
- **Datasets root:** `CQLITE_DATASETS_ROOT=/home/dev/projects/cqlite/test-data/datasets` for every bench, profile and test run.
- **Cargo target dirs:** source `round2/env.sh` (which sources `experiment/env.sh`) and build only with `msx_cargo <name> …` into `work/target/<name>`. Never build into `/home/dev/.cache-dev/cargo-target`.
- **Toolchain:** Rust `1.97.1`, pinned by `round2/rust-toolchain.toml`. CQLite's own pin is respected inside the clone; if it differs from 1.97.1, record that in `round2/DECISIONS.md` and use CQLite's.
- **Round 1 artifacts are read only:** `experiment/` (all of it), `tools/`, `work/target/tool/release/msx` (sha256 `9007b7ac…`), `docs/results/2026-11-llm-asm-experiment.md`, `work/cqlite`. Round 2 copies what it reuses.
- **No public-API, on-disk-format or wire change to CQLite** (spec section 8). Proposals only.
- **No system package installs, no pushes, no merges, no PRs.** Commit to machsmith's local `main` at the end of every task. Ask the owner before pushing.
- **Never build or test during a measurement window.** `r2 run` holds `work/r2.lock` for the whole run; `round2/build.sh` and `round2/test.sh` refuse to start while it is held.
- **Ledger is append-only.** Never edit or delete a line of `round2/results/ledger.jsonl`.
- **Honesty:** only confirmation, cross-check, null and calibration runs are reported as results. Screening runs appear only in an attempts table labelled provisional. No claim smaller than T. No "CQLite is X% faster".
- **Stop-and-ask gates** (stop, tell the owner, wait): spec section 10, plus any step that would need to change `/home/dev/projects/cqlite` or round 1's artifacts.

## Verified on this host (12 November 2026)

| Fact | Evidence | Used in |
|---|---|---|
| CQLite HEAD is `7f9abe36…`, 3 commits after round 1's `2e5b7a4c` | `git log` in the CQLite repo | Task 1 |
| CQLite uses Criterion 0.5.1; benches are `harness = false` targets `read`, `decode`, `partition_lookup` | `Cargo.lock`, `cqlite-core/Cargo.toml` | Tasks 4, 5 |
| `[profile.bench]` has `debug = true`; `perfsym`/`perfprof` profiles exist | CQLite `Cargo.toml` | Tasks 6, 7 |
| `IndexReader::key_lookup` is `pub key_lookup: HashMap<Arc<[u8]>, usize>` in `pub mod index_reader` | `cqlite-core/src/storage/sstable/index_reader/mod.rs:134` | Task 11 |
| `rustc-hash = "1.1"` is already a `cqlite-core` dependency | `cqlite-core/Cargo.toml:132` | Task 11 |
| CQLite's gate is `scripts/agent-gate.sh`, run detached via `scripts/flow/gate-detached.sh`; it compares against `origin/main` | CQLite `CLAUDE.md` | Task 8 |
| Host has 32 threads; SMT sibling of core `c` is `c + 16` | round 1 `measure.rs` `SIBLING_OFFSET` | Task 5 |
| `tools/profile-cqlite.sh` has its `perf script --show-mmap-events` lines misplaced inside the "no binary" error branch | read directly | Task 7 |

## File map

```
round2/
  env.sh                     # sources experiment/env.sh; R2_* vars
  rust-toolchain.toml        # 1.97.1
  Cargo.toml                 # workspace: members = ["referee"]
  referee/
    Cargo.toml
    src/lib.rs
    src/stats.rs             # copied from experiment/harness/src/stats.rs, adapted (Task 2)
    src/criterion.rs         # parse a CRITERION_HOME tree (Task 3)
    src/host.rs              # host snapshot, quiet wait, session monitor (Task 4)
    src/ledger.rs            # append-only ledger (Task 4)
    src/session.rs           # one session: run both arms, collect stats (Task 5)
    src/run.rs               # a run: sessions, rejections, verdicts, record (Task 5)
    src/bin/r2.rs            # CLI (Task 5)
    tests/fixtures/…         # criterion trees, fake bench script
  build.sh                   # rev + bench -> pinned binary (Task 6)
  test.sh                    # CQLite gate / fallback on a worktree (Task 6)
  profile.sh                 # perf record per case (Task 7)
  attribute.py, test_attribute.py   # shares, call-tree buckets, per-case (Task 7)
  DECISIONS.md               # decision ledger (spec section 11), kept current
  results/
    ledger.jsonl
    profile/…                # Task 9
    ruler-check.md           # Task 10
    attempts.md              # screening runs, provisional (Task 11)
docs/rules/<rule-id>.md, docs/rules/proposals/<rule-id>.md, docs/rules/README.md
docs/results/2026-11-round-2-cqlite-hotspots.md
```

## Protocol constants (`referee/src/run.rs`, copied verbatim from the spec)

| Constant | Value | Spec |
|---|---|---|
| `CONFIRM_SESSIONS` | 20 | 6.2 |
| `SCREEN_SESSIONS` | 5 | 6.2 |
| `CROSSCHECK_SESSIONS` | 10 | 6.2 |
| `NULL_SESSIONS` | 20 per workload | 6.3 |
| `MAX_SESSION_REPLACEMENTS` | 3 (4th rejection → `NOISE_GATE_FAILED`) | 6.4 |
| `MAX_CONFIRM_TRIES` | 3 per branch | 6.4 |
| `THRESHOLD_FLOOR` | 0.05 | 6.3 |
| `NULL_BAND_MAX` | 0.05 (three tries over → ruler broken) | 6.3 |
| `NULL_MAX_AGE_DAYS` | 7 | 6.3 |
| `BOOTSTRAP_SEED` / `SESSION_SEED_BASE` | `0x6D7378` | 6.2 |
| `BOOTSTRAP_RESAMPLES` | 10,000 | 6.2 |
| Criterion args | `--bench --warm-up-time 1 --measurement-time 3 --sample-size 50 --noplot` | 6.2 |
| Quiet precondition | no `rustc`/`cargo` process, 1-min load < 2.0, held 10 min | 6.4 |
| Session rejection | 1-min load > 3.0; any `rustc`/`cargo` seen; SMT sibling util > 0.20; > 50 involuntary ctx switches per second of bench-process run time | 6.4 |
| Target-case share floor | 3% | 6.5 |
| Profile duration | `--profile-time 20` per case; two profiles at different times; "confirmed" = share ≥ 3% in both and differs by < 2 pp | 5 |
| Max fixes / attempts | 4 fixes; 2 screened attempts each | 7.1 |

## Shared interfaces

All in crate `r2_referee` (`round2/referee`).

```rust
// stats.rs (Task 2)
pub struct Interval { pub point: f64, pub lo: f64, pub hi: f64 }
pub fn median(v: &mut [f64]) -> f64;
pub fn geomean(v: &[f64]) -> f64;
pub fn bootstrap_ci(session_ratios: &[f64]) -> Interval;          // unchanged from round 1
pub fn band_of(iv: &Interval) -> f64;                              // max(1-lo, hi-1)
pub fn threshold(band: f64) -> f64;                                // max(0.05, 2*band)
pub fn verdict(iv: Interval, t: f64) -> Verdict;
pub enum Verdict { Won, Lost, Tied, Inconclusive }                 // serializes "WON" etc.
/// sessions[s][case] = cand/base ratio for that session; returns the bootstrap interval of the
/// per-session geometric mean over `cases`. Errors if any session lacks any case.
pub fn set_interval(sessions: &[BTreeMap<String, f64>], cases: &[String]) -> Result<Interval, String>;

// criterion.rs (Task 3)
pub struct CaseStat { pub id: String, pub median_ns: f64, pub mean_ns: f64, pub mad_ns: f64, pub samples: usize }
/// Reads every `<home>/**/new/benchmark.json` + `sample.json`; keys by benchmark.json `full_id`.
pub fn read_home(home: &Path) -> Result<BTreeMap<String, CaseStat>, String>;

// host.rs (Task 4)
pub struct HostSnapshot { pub at_unix: u64, pub load1: f64, pub load5: f64, pub load15: f64,
                          pub cargo_rustc_pids: Vec<u32>, pub core: Option<usize> }
pub trait Probe { fn loadavg(&self) -> (f64, f64, f64); fn cargo_rustc_pids(&self) -> Vec<u32>;
                  fn cpu_times(&self) -> BTreeMap<usize, (u64, u64)>; }
pub struct ProcProbe;                                              // real /proc implementation
pub fn snapshot(p: &dyn Probe, core: Option<usize>) -> HostSnapshot;
pub fn quiet_wait(p: &dyn Probe, hold: Duration, max_load: f64, timeout: Duration,
                  sleep: &mut dyn FnMut(Duration)) -> Result<HostSnapshot, String>;
pub fn choose_core(p: &dyn Probe) -> usize;                        // round 1's rule, cores 2..=15
pub struct SessionMonitor; // start(core), sample() every 1 s, finish() -> Option<Rejection>
pub enum Rejection { Load(f64), CargoRustc(Vec<u32>), SiblingUtil(f64), CtxSwitches(f64), ProcessFailed(String), MissingCase(String) }

// ledger.rs (Task 4)
pub fn append(path: &Path, kind: &str, body: serde_json::Value) -> Result<String, String>; // returns "R2-000001"
pub fn read_all(path: &Path) -> Result<Vec<serde_json::Value>, String>;
pub fn machsmith_state() -> serde_json::Value;                     // {"commit", "dirty_paths": [...]}

// session.rs / run.rs (Task 5)
pub struct Arm { pub name: String, pub bin: PathBuf, pub sha256: String }
pub struct RunSpec { pub kind: RunKind, pub workload: String, pub base: Arm, pub cand: Arm,
                     pub sessions: u32, pub targets: Vec<String>, pub rule: Option<String>,
                     pub null_ref: Option<String> }
pub enum RunKind { Null, Calibration, Screen, Confirm, Crosscheck, Proposal }
pub fn run(spec: &RunSpec, env: &RunEnv) -> Result<String /* ledger id */, String>;
```

CLI (`r2`):

```
r2 quiet-wait [--hold-min 10] [--max-load 2.0] [--timeout-h 12]
r2 run --kind <null|calibration|screen|confirm|crosscheck|proposal> --workload <read|decode|partition_lookup>
       --base <bin> --cand <bin> [--sessions N] [--targets <file.json>] [--rule <id>] [--null <ledger-id>]
r2 report <ledger-id>          # markdown to stdout
r2 nullband --null <ledger-id> --workload W [--targets <file.json>]   # prints b and T for a target set
```

## Task index

| # | Task | Phase |
|---|---|---|
| 1 | Clone, scaffolding, env, decision ledger | 1 |
| 2 | `stats.rs`: round 1 statistics + set intervals | 1 |
| 3 | `criterion.rs`: read a Criterion home | 1 |
| 4 | `host.rs` + `ledger.rs` | 1 |
| 5 | `session.rs`, `run.rs`, `r2` CLI | 1 |
| 6 | `build.sh`, `test.sh` | 1 |
| 7 | `profile.sh`, `attribute.py` | 1 |
| 8 | Base commit test suite | 2 |
| 9 | Profiles ×2 and hot-spot ranking | 2 |
| 10 | Null run and ruler calibration | 2 |
| 11 | Per hot spot (≤4 fixes): rule → fix → gate → confirm → cross-check | 3 |
| 12 | Proposal sizing (if any, time permitting) and write-up | 4 |
| 13 | Final review on the most capable model; owner message | 4 |

## Review Focus

Inputs or failure modes the spec implies but would not otherwise be tested; each has a test in the named task.

1. **A case present in one arm but not the other** (CQLite skip-registers cases when an optional corpus such as `test_da` is absent). Expected: the session is rejected with `MissingCase`, never a ratio from a partial set. Test in Task 5.
2. **A bench process that exits non-zero or panics mid-session.** Expected: session rejected with `ProcessFailed`, counted against the 3 replacements; stderr tail recorded. Test in Task 5.
3. **Case ids containing `/`, spaces or characters Criterion rewrites in directory names.** Expected: ids come from `benchmark.json`'s `full_id`, not directory names, and target-case files match on `full_id`. Test in Task 3.
4. **Baseline and candidate binaries with the same sha256 in a `confirm`, `screen` or `crosscheck` run.** Expected: refused before any session (only `null` may compare a binary with its copy). Test in Task 5.
5. **A run killed partway (Ctrl-C, OOM, session timeout).** Expected: an `aborted` ledger record with the sessions completed so far and no verdict; the lock is released. Test in Task 5.
