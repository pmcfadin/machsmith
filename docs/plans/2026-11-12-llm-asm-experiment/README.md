# LLM Assembly vs Optimizer Experiment: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Measure, on three real CQLite kernels in isolation, whether LLM-written assembly beats rustc's best output, and write up a verdict per kernel.

**Architecture:** A small Rust workspace in `machsmith/experiment/`. Every contestant (each compiler build of CQLite's code, each candidate) is a separate shared object exporting the same C-ABI kernel symbol. One harness binary, `msx`, loads them, checks candidates against the reference in a child process, measures them with in-process hardware counters over replayed real inputs, and appends every result to a ledger. Fresh agents write candidates under a fixed attempt budget; the orchestrator runs one final exam per kernel and writes the report.

**Tech Stack:** Rust 1.97.1 (stable), `libc`, `serde`/`serde_json`, `sha2`, `rand_chacha`; `gcc`/GNU `as` for assembling candidates; `perf` for the one-time profile; `llvm-profdata`, `llvm-objdump`, `llvm-nm`, `llvm-readobj` from the rustup `llvm-tools` component; Python 3 for one profile-aggregation script.

**Spec:** `docs/specs/2026-11-12-llm-asm-experiment-design.md`. Read it first. This plan argues from it and does not repeat its reasoning.

## Documents

| File | Contents |
|---|---|
| `README.md` (this file) | Constraints, file map, shared interfaces, protocol constants, task index |
| `phase-1-profile-and-kernels.md` | Tasks 1-4: repo, CQLite profile and ceiling, kernel crate, traces |
| `phase-2-harness.md` | Tasks 5-10: loader, builds, lint and checker, measurement, commands, self-test |
| `phase-3-contest-and-writeup.md` | Tasks 11-14: null gate, contest, final exam, write-up |
| `candidate-agent-prompt.md` | The prompt template for the six candidate-writing agents |

Do the tasks in order. Phase 2 tasks depend on each other's interfaces, listed in section "Shared interfaces" below.

## Global Constraints

- **Do not modify `/home/dev/projects/cqlite`.** No edits, no builds inside it, no `git worktree add` from it. Work on a local clone at `machsmith/work/cqlite`. Reading its files and its `test-data/` is fine.
- **Datasets root:** `CQLITE_DATASETS_ROOT=/home/dev/projects/cqlite/test-data/datasets`, the original repository's directory, read-only. The clone's `test-data/datasets` is incomplete (the SSTables are fetched, not tracked) and benches fail or list no cases against it.
- **CQLite commit:** `2e5b7a4cf6abe6f51b7677ecb7199e2cab087c0e`. The clone is checked out at exactly this commit.
- **Always set `CARGO_TARGET_DIR` explicitly.** The environment exports a shared `CARGO_TARGET_DIR=/home/dev/.cache-dev/cargo-target` used by other agents. Every cargo invocation in this plan sets its own under `machsmith/work/target/…`. Never build into the shared one.
- **Toolchain:** Rust `1.97.1`, pinned by `experiment/rust-toolchain.toml`. Stable only: no nightly, no `-Z` flags.
- **Target:** x86-64 Linux, this host only (AMD Ryzen AI MAX+ 395, Zen 5). `-C target-cpu=znver5` for rungs 2 and 3.
- **No new system packages.** If a tool is missing, stop and report; do not install with a package manager. `cargo install` into the user's cargo home is not needed by this plan.
- **Shared host.** All 32 threads are shared with other agents. Measurements are serialized through a file lock (Task 9). Never run two measurements at once. Never run a build while a final-exam or null measurement is running.
- **No Incus VM, no infrastructure tickets** for this experiment.
- **Git:** `machsmith` becomes its own git repository in Task 1. Commit at the end of every task. Do not push anywhere.
- **Ledger is append-only.** Never edit or delete a line of `experiment/results/ledger.jsonl`.
- **Honesty rule:** the write-up reports only final-exam results. Search-phase numbers appear only in the attempts history, labeled provisional.
- **Stop-and-report gates** (stop work, tell the owner, wait):
  1. The null band exceeds 5% on three tries at different times (Task 11).
  2. Any step would require changing `/home/dev/projects/cqlite`.
  3. A row of "Verified on this host" below no longer holds (for example the toolchain changed).

## Verified on this host (12 November 2026)

These were probed before the plan was finalized. Treat them as facts; each still has a test in the task named, so a change is caught.

| Fact | Evidence | Used in |
|---|---|---|
| `-C llvm-args=-align-all-functions=6` is accepted by rustc 1.97.1 and aligns exported functions to 64 bytes | probe `cdylib`: `msx_vint` at `…c7c0`, `…c700`, `…c100` across three builds | Task 6 |
| PGO works through a `dlopen`ed `cdylib` | instrumented library loaded by a host program wrote a `.profraw` at normal exit; `llvm-profdata merge` and `-C profile-use` succeeded and changed the kernel's code | Task 6 |
| `llvm-profdata show --all-functions --counts` prints `Block counts: […]` per function and no `Function count:` line | observed output | Task 6 (`verify_profile`) |
| A function never called during training has all-zero block counts | `msx_vint` when only `msx_vint_chain` was driven | Task 6 |
| Rust `no_std` `staticlib` + `gcc -shared -nostdlib -nostartfiles` links, with no `NEEDED` entries and no init sections | probe build | Task 6, Task 7 |
| That link leaves `memcpy` undefined (one `R_X86_64_GLOB_DAT`) when the code copies a variable number of bytes | `llvm-nm -u`, `llvm-readobj --dyn-relocations` | Task 7 (lint allowance) |
| Profile-use on the `no_std` build warns `no profile data available for function …rust_begin_unwind` | observed; harmless, the panic handler exists only in that build | Task 6 |
| rustc compiles `decode_unsigned` to code that calls `memcpy` through the GOT, at r1, r2 and r3, and in CQLite's own fat-LTO bench binary | disassembly | Tasks 3, 7, 10 |
| `perf_event_open` counts `cycles`, `instructions`, `branch-misses` for the calling thread, user-only, as one group, without multiplexing | C probe: 60,000,020 and 60,000,019 instructions on two runs of the same loop | Task 8 |
| `/proc/stat` has per-CPU lines for all 32 threads; `randomize_va_space` is 2; `xgetbv1` is in the CPU flags; `scaling_cur_freq` is readable | read directly | Tasks 5, 8 |
| GNU `as` is 2.42, `gcc` is 13.3.0 | `--version` | Task 6 |
| CQLite benches build with `--profile perfprof --features cli-helpers,bench-internals` in about two minutes and run with `--bench --profile-time N`; `--bench --list` lists cases | scratch clone | Tasks 2, 4 |
| `nom` in CQLite's lockfile is 7.1.3; `parse_vuint` is a thin adapter over `decode_unsigned` returning `ErrorKind::Eof` on failure | read | Task 3 |
| 151 of 152 BIG `Index.db` files replay as a clean chain, 4,629 entries in total | independent Python walk | Task 4 |

## Review Focus

Failure modes the spec implies that are most likely to bite. Each has a test in the task named.

1. **A build lands in the shared cargo target directory** and collides with another agent's build. Expected: `env.sh` clears the inherited value, `msx_cargo` always sets one under `machsmith/work/`, and `msx_require_env` fails fast on any other value. Test in Task 1.
2. **A candidate over-reads only at certain alignments or lengths** (for example an 8-byte load near the end of a buffer). Expected: caught as `OOB_OR_CRASH`. Test in Task 7 with the `overread1` sabotage candidate.
3. **PGO silently does nothing** because the profile was never written by a `dlopen`ed library or did not match. Expected: the rung-3 build fails unless the merged profile shows a nonzero entry count for the kernel symbol. Test in Task 6.
4. **A noisy sample is accepted** when the thread was preempted or counters were multiplexed. Expected: the sample and its block are discarded and counted. Test in Task 8 with synthetic samples.
5. **A candidate shared object runs code at load time** (an initializer) inside the judging process. Expected: rejected by the lint before any `dlopen`. Test in Task 7 with the `ctor` sabotage candidate.
6. **A trace does not replay cleanly** (an index file whose entries do not end exactly at end of file). Expected: that file is excluded and listed in the manifest, not silently truncated. Test in Task 4.

## File map

```
machsmith/
  .gitignore                         work/ and build outputs
  README.md                          what this repo is; points at docs/
  docs/
    specs/…                          spec (exists)
    plans/2026-11-12-llm-asm-experiment/…   this plan
    results/2026-11-llm-asm-experiment.md   write-up (Task 14)
  experiment/
    rust-toolchain.toml              pins 1.97.1
    Cargo.toml                       workspace: kernels, harness
    env.sh                           sourced by every script; sets and checks CARGO_TARGET_DIR, tool paths
    kernels/                         crate msx-kernels (cdylib): CQLite code copied verbatim + C shims
      src/lib.rs
      src/vint.rs                    decode_unsigned + msx_vint + msx_vint_chain
      src/index_framing.rs           parse_big_index_entry_framing + deps + shims
      SELECTION.md                   why these three, with profile figures
      src/bti_sparse_find.rs         BtiNode::find_child, Sparse arm, + shims
      CARDS/<kernel>.md              kernel card: contract, ABI, postconditions (given to agents)
    harness/                         crate msx-harness (lib + bin `msx`)
      src/lib.rs
      src/spec.rs                    KernelSpec trait, OutRec, registry
      src/spec_vint.rs, spec_index_framing.rs, spec_bti_sparse_find.rs
      src/loader.rs                  dlopen wrapper
      src/guard.rs                   guard-page buffers
      src/tramp.rs                   ABI-checking call trampoline (global_asm!)
      src/check.rs                   differential checker (parent/child)
      src/lint.rs                    static lint of candidate source and shared object
      src/counters.rs                perf_event_open group
      src/measure.rs                 sessions, blocks, sample hygiene
      src/stats.rs                   ratios, bootstrap, verdicts
      src/build.rs                   rung builds, candidate builds
      src/ledger.rs                  append-only JSONL
      src/bin/msx.rs                 CLI
      tests/…                        integration tests
    candidate-wrapper/               tool-owned no_std wrapper crate for Rust candidates
    selftest/<kernel>/…              tool-owned sabotage and control candidates (Task 10)
    trace-capture/cqlite-trace.patch instrumentation patch applied to the clone only
    traces/<kernel>/search/*.seg     recorded inputs for search (committed)
    traces/<kernel>/holdout/*.seg    recorded inputs for the final exam (committed; agents must not read)
    traces/<kernel>/MANIFEST.json
    candidates/<kernel>/BRIEF/       what agents are given (Task 12)
    candidates/<kernel>/<arm>/       agents write here: attempt-NN.s or attempt-NN.rs, NOTES.md, FINAL.md
    results/ledger.jsonl
    results/profile/                 ceiling report (Task 2)
    results/null/, results/final/    JSON outputs
  tools/
    profile-cqlite.sh, ceiling.py    Task 2
  work/                              gitignored: cqlite clone, cargo target dirs, built shared objects
```

## Shared interfaces

These names are used across tasks. Do not rename them.

### Kernel C ABIs (crate `msx-kernels`, and every candidate)

```rust
// vint
#[no_mangle] pub unsafe extern "C" fn msx_vint(ptr: *const u8, len: usize, out: *mut u64) -> usize;
// Returns bytes consumed (1..=9) and writes the value to *out.
// Returns 0 if the input is empty or shorter than the lead byte declares; *out is not written.

// index_framing
#[repr(C)] pub struct FramingOut { pub framing_len: u64, pub promoted_len: u64 }
#[no_mangle] pub unsafe extern "C" fn msx_index_framing(ptr: *const u8, len: usize, out: *mut FramingOut) -> u32;
// Returns 0 on success and writes *out.
// On a nom Error/Failure returns 1 + (nom::error::ErrorKind as u32); on Incomplete returns u32::MAX. *out is not written.

// bti_sparse_find
#[repr(C)] pub struct Transition { pub byte: u8, pub child: u64 }      // size 16, align 8; padding bytes unspecified
#[no_mangle] pub unsafe extern "C" fn msx_bti_sparse_find(t: *const Transition, n: usize, byte: u32, out: *mut u64) -> u32;
// 1 if a transition with .byte == (byte as u8) exists (writes its child to *out), else 0 (*out not written).
// `t` is sorted ascending by .byte with no duplicates. Only the low 8 bits of `byte` are meaningful.
```

Only `A` builds also export the report-only inline arm:

```rust
#[no_mangle] pub unsafe extern "C" fn msx_<kernel>_chain(seg: *const u8, seg_len: usize, checksum: *mut u64) -> u64; // returns ops
```

All kernels may read only `[ptr, ptr+len)` and write only `*out`.

### Harness types (`harness/src/spec.rs`)

```rust
pub type RawFn = *const core::ffi::c_void;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
#[repr(C)]
pub struct OutRec { pub ret: u64, pub out: [u64; 3], pub out_written: u64 }   // out_written: 0 or 1

pub trait KernelSpec: Sync {
    fn name(&self) -> &'static str;                 // "vint"
    fn symbol(&self) -> &'static str;               // "msx_vint"
    fn chain_symbol(&self) -> &'static str;         // "msx_vint_chain"
    /// Call the kernel once through the ABI trampoline. `input` is exactly the bytes it may read.
    unsafe fn run_case(&self, f: RawFn, input: *const u8, len: usize, aux: u64) -> Result<OutRec, AbiViolation>;
    /// Drive the kernel over one trace chunk as CQLite would. Returns (ops, checksum of outputs).
    unsafe fn run_chain(&self, f: RawFn, chunk: &[u8]) -> (u64, u64);
    /// Split a segment into chunks of at least `min_ops` operations, using the reference to find boundaries.
    unsafe fn chunks<'a>(&self, reference: RawFn, seg: &'a [u8], min_ops: u64) -> Vec<&'a [u8]>;
    /// Enumerate every individual call in a segment as (input bytes, aux).
    unsafe fn trace_cases(&self, reference: RawFn, seg: &[u8], sink: &mut dyn FnMut(&[u8], u64));
    /// Generated structural edge cases, deterministic.
    fn edge_cases(&self, sink: &mut dyn FnMut(&[u8], u64));
    /// `n` seeded random cases.
    fn random_cases(&self, seed: u64, n: u64, sink: &mut dyn FnMut(&[u8], u64));
}
pub fn kernel(name: &str) -> Option<&'static dyn KernelSpec>;
pub fn all_kernels() -> &'static [&'static dyn KernelSpec];
```

### Built artifacts (all under `machsmith/work/build/`)

| Path | What |
|---|---|
| `A/r1/libmsx_kernels.so`, `A/r2/…`, `A/r3/…`, `A/r3copy/…` | compiler builds |
| `A/reasm/<kernel>.so` | A.r3's assembly through the candidate path, where producible (Task 10) |
| `cand/<kernel>/<arm>/attempt-NN/cand.so` | assembly candidate, or Rust candidate built with the r3 recipe |
| `cand/<kernel>/rust/attempt-NN/cand-r2.so` | Rust candidate built with the r2 recipe (final exam only) |

### CLI (`msx`, run from `machsmith/experiment/`)

| Command | Purpose | Task |
|---|---|---|
| `msx rungs build` | build `A.r1`, `A.r2`, `A.r3`, `A.r3copy` | 6 |
| `msx check --kernel K --so PATH --phase search\|final [--seed-from FILE]` | correctness only | 7 |
| `msx measure --kernel K --protocol search\|final --split search\|holdout --contestants name=path,…` | measurement only | 8 |
| `msx null --kernel K` | the ruler check | 9 |
| `msx attempt --kernel K --arm asm\|rust --file PATH` | lint, build, check, measure, ledger | 9 |
| `msx final --kernel K` | final exam for the finalists named in `FINAL.md` | 9 |
| `msx selftest` | run the sabotage and control candidates | 10 |
| `msx report` | Markdown tables from the ledger | 14 |

Every command prints one JSON object on stdout and human-readable progress on stderr. Exit code 0 means the command ran, whatever the verdict; nonzero means the tool itself failed.

### Verdict strings

Check: `LINT_FAILED`, `BUILD_FAILED`, `INCORRECT`, `OOB_OR_CRASH`, `ABI_VIOLATION`, `TIMEOUT`, `PASS`.
Measure, per comparison: `WON`, `LOST`, `TIED`, `INCONCLUSIVE`, or `NOISE_GATE_FAILED` for the whole run.

## Protocol constants

Copied from the spec and fixed here. They live in one file, `harness/src/measure.rs` (measurement) and `harness/src/check.rs` (correctness), as named constants.

| Constant | Value |
|---|---|
| `SAMPLE_MIN_OPS` | 50,000 operations per sample |
| `SMALL_TRACE_MIN_CHUNKS` | 20: a split that yields fewer chunks than this at `SAMPLE_MIN_OPS` is measured in small-trace mode (Task 8) |
| `SEARCH_SESSIONS` × `SEARCH_BLOCKS` | 6 sessions × 60 blocks |
| `FINAL_SESSIONS` × `FINAL_BLOCKS` | 30 sessions × 200 blocks |
| `MAX_DISCARD_FRACTION` | 0.30 of a session's blocks; above it the session is rerun |
| `MAX_SESSION_RERUNS` | 3 per run, then `NOISE_GATE_FAILED` |
| `INSTR_TOLERANCE` | 0.0001 relative difference from the contestant's modal instruction count on that chunk |
| `BOOTSTRAP_RESAMPLES` | 10,000, percentile method, 95% interval, RNG seed `0x6D7378` |
| `THRESHOLD_FLOOR` | 0.05 |
| `NULL_BAND_MAX` | 0.05 |
| `ENTRY_ALIGN` | 64 bytes |
| `RANDOM_CASES_SEARCH` | 2,000,000, seed `0x5EED0001` |
| `RANDOM_CASES_FINAL` | 20,000,000, seed = first 8 bytes (little-endian) of SHA-256 of the candidate source file |
| `CHECK_TIMEOUT_SECS` | 300 wall-clock; child CPU rlimit 240 s |
| `RODATA_MAX` | 4,096 bytes for assembly candidates, 16,384 for Rust candidates |
| `TEXT_MAX` | 8,192 bytes of code in a candidate's kernel function |
| `COUNTED_ATTEMPTS` | 10 per agent |
| `MAX_SUBMISSIONS` | 30 per agent |
| `HOLDOUT_FRACTION` | about 30% of recorded segments, chosen by a hash of each segment's content (Task 4) |

## Task index

| # | Task | Phase doc |
|---|---|---|
| 1 | Repository, workspace skeleton, environment guard | 1 |
| 2 | Clone CQLite, profile it, ceiling report | 1 |
| 3 | `msx-kernels` crate for the three fixed kernels, kernel cards | 1 |
| 4 | Trace capture, search/holdout split, manifest | 1 |
| 5 | Loader, guard buffers, ABI trampoline | 2 |
| 6 | Rung builds with PGO, candidate builds, alignment | 2 |
| 7 | Static lint and differential checker | 2 |
| 8 | Counters, measurement sessions, statistics | 2 |
| 9 | Ledger, `attempt`, `null`, `final` commands | 2 |
| 10 | Self-test: sabotage and control candidates | 2 |
| 11 | Run the ruler check | 3 |
| 12 | Run the contest (six agents) | 3 |
| 13 | Final exams | 3 |
| 14 | Write-up | 3 |

## Self-review notes (plan author)

- Spec sections 3 to 9 each map to tasks: contestants (6, 10), kernels (3), ceiling (2), correctness (5, 7), measurement (8, 9, 11), contest (9, 12, 13), deliverable (14). Limitations (spec 10) are required text in Task 14.
- The spec's `A.inline` arm is built in Task 3 and measured in Task 13.
- The kernels, their ABIs, the build recipes and the capture method are fixed from evidence gathered on 12 November 2026 (spec section 5.1 and the table above). What is left to the implementer is the internal structure of each module.
