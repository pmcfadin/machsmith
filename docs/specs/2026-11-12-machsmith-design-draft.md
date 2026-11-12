# machsmith: design draft (pre-review)

> **Superseded for now (12 November 2026).** The owner split the work in two: an
> experiment first (`2026-11-12-llm-asm-experiment-design.md`), then a redesign
> of this toolkit using its results and the council review findings recorded in
> section 12 of that document. Do not implement this draft.

Status: DRAFT for council review, 12 November 2026. Not yet approved by the owner.
Nothing is built. The repo contains only this document.

## 1. Intent

Question the owner wants answered: *how do we know the machine code rustc/LLVM
produced is the best code for our workload and target CPU?*

machsmith is a toolkit of instruments that AI coding agents use to answer that
question for Rust code, and to push the code further when the answer is "it
isn't". The agent does the optimizing. The tools measure, verify and explain,
and are meant to be honest by construction: a tool cannot report "faster"
without passing the gates below.

There is no known LLVM miss motivating this. A well-evidenced "no meaningful
headroom here" is a valid, successful result.

First proving ground: CQLite (`/home/dev/projects/cqlite`), a Rust reader for
Cassandra SSTables. Its parsing, decoding and index traversal are tight loops
with heavy branching and memory access.

## 2. Decisions made by the owner

### 2.1 Instrument, not optimizer
Primary output is a measured answer (how much headroom, where, and why).
Faster code is a byproduct of a positive finding.

### 2.2 Core comparison: compiler assembly (A) vs optimized assembly (B)
B is assembly written to beat A for the workload and CPU. Rust rewrites,
compiler flags and LLVM changes are ways to explain or reproduce a win
afterwards. They are not the source of candidates.

### 2.3 Who writes B
Mixed. An LLM agent is the main generator: it reads A plus the profile and
writes replacement assembly, iterating on measured results. The harness accepts
candidates from any source (agent, search tool, human) and judges them the same
way. STOKE-style stochastic search is a later addition, not v1.

### 2.4 Unit of replacement
Two levels:
- **Search level:** a chosen kernel is given a forced boundary
  (`#[inline(never)]`) in a measurement build so it is a clean standalone
  function. Candidates are searched and tested here.
- **Gate level:** the win must survive in the real release binary by replacing
  a symbol that actually exists after inlining (link-time swap), measured end
  to end. Nothing counts as a finding without this.
Replacing a region inside a function (binary patching) is out of scope for v1.

Context: CQLite's release profile is fat LTO, `codegen-units=1`, opt-level 3,
`panic=abort`, so small hot functions are mostly inlined away.

### 2.5 Correctness bar for B
Required:
- differential testing against A (A is the reference): same return value,
  output bytes, error kind, buffer side effects, on fixtures, real SSTable
  values and boundary cases;
- differential fuzzing for a fixed budget (CQLite already has fuzz targets:
  `fuzz_vint`, `fuzz_value_decode`, `fuzz_block_emit`, `fuzz_bti`, ...);
- the full CQLite test suite in the symbol-swap build.
Also checked: calling convention (callee-saved registers, stack alignment) and
out-of-bounds reads (guard pages / sanitizers). A candidate that reads out of
bounds is rejected even if its answers are right.
Formal equivalence (symbolic checking of short loop-free kernels) is
opportunistic. Each result is labeled "proven" or "tested only".

### 2.6 What "faster" means
- Primary metric: cycles per operation from hardware counters. Wall-clock is
  always reported beside it.
- Method: A and B run paired and interleaved, pinned to the same core, many
  repetitions. Report the B/A ratio with a bootstrap confidence interval.
- Thresholds (to be recalibrated after noise is measured): >=5% on a kernel
  with the interval clear of zero; >=1% end to end in the real binary, or at
  least no regression for a small kernel.
- A-vs-A noise gate first: if the harness cannot show A ~= A within about 1%,
  it reports "can't tell" and nothing about B counts.

### 2.7 What A is: a baseline ladder
B is measured against several As:
1. the shipped release profile;
2. plus `-C target-cpu=znver5`;
3. plus PGO trained on the same workload;
4. (later) plus BOLT.
Each gap is labeled by the lowest rung that closes it: "flag", "PGO", or
"real headroom" (survives the top rung).

### 2.8 After a real finding
Escalating chain, labeled by where it stops:
1. report: assembly diff, measurements, counter breakdown, correctness evidence;
2. the agent tries Rust source rewrites that make rustc emit B-like code,
   measured on the same ladder ("explained by a Rust rewrite", with a patch);
3. (later) minimize to an LLVM IR reproducer and draft a missed-optimization
   report.
Shipping B's assembly into the target project is never automatic.

### 2.9 Shape: toolkit of primitives plus a reference loop
CLI primitives with `--json` output:
- `hotspots`: profile and rank hot code for a workload;
- `isolate`: produce the forced-boundary build of a kernel;
- `asm`: dump A with source mapping and counter annotations;
- `ab`: correctness plus paired-measurement verdict for A vs a candidate;
- `ladder`: run a candidate against each compiler rung;
- `swap`: end-to-end symbol-swap check in the real binary;
- `explain`: Rust-rewrite attempt record (LLVM reproducer support later).
A skill document teaches agents the workflow. An MCP wrapper is optional later.

### 2.10 Reference agent loop
Claude Code running headless, following the machsmith skill, with a fixed
budget per kernel. machsmith contains no LLM code and depends on no one agent.
machsmith logs every `ab`/`ladder` call itself, so search-cost figures come
from the tool's records.

### 2.11 Workloads
Each target crate declares workloads in a spec file: named corpora, how to
drive them, weights. A finding is always tied to a named workload and checked
for regressions on the others. CQLite gets at least three: cache-resident
(existing ~7.5 MB fixtures), a large sequential scan well beyond L3 (64 MiB),
and a point-lookup-heavy one. Larger corpora must be generated (CQLite has
`tools/ws0-corpus-gen` and perf-corpus manifests, but the data is not on disk).

### 2.12 Trusting a "no"
- Seeded calibration suite: kernels with planted slowdowns (an unremovable
  bounds check, a loop that fails to vectorize, a branchy varint decoder with a
  known branchless alternative, a needless copy), some in copies of real CQLite
  functions. The reference loop must recover an agreed fraction within budget.
  The suite doubles as machsmith's regression test.
- Coverage report with every "no": candidates generated, how many passed
  correctness, budget spent, kinds of change attempted.
- Wording: never "optimal". "No improvement >= threshold found within budget X
  for these kinds of change", with the latest calibration score.
- Known LLVM/rustc missed-optimization issues are added as calibration cases
  when good ones are found.

### 2.13 Minotaur / Alive2
Off the v1 critical path. A slot is left in `explain` for IR-level checkers
later. Reason: Minotaur works on LLVM IR and mostly SIMD, and builds against a
patched LLVM main while rustc 1.97.1 ships LLVM 22.1.6.

### 2.14 Implementation
All Rust: a `machsmith` CLI crate plus harness crates it generates into the
target's build (differential driver, measurement loop reading counters
in-process via `perf_event_open`, B linked in via `global_asm!`). perf and the
`llvm-tools` are called, not reimplemented. Separate repo at
`/home/dev/projects/machsmith`; CQLite is an external target.

### 2.15 Measurement environment
Start on the shared host with `taskset` pinning and the noise gate; record
load, frequency and pinned cores with every result. A request for reserved
cores will be drafted separately. The VM is not used for benchmarking.

### 2.16 v1 scope and finish line
- x86-64 only, tuned and measured on the Zen 5 host.
- `hotspots`, `isolate`, `asm`, `ab`, `ladder`, `swap` working end to end on
  CQLite; `explain` is a stub that records Rust-rewrite attempts.
- All gates working; calibration suite at an agreed recovery rate.
- Reference loop run on CQLite's top 5 hot kernels across the three workloads.
- Finish line: a written case study with a verdict per kernel ("flag", "PGO",
  "real headroom X%: Rust rewrite / unexplained", or "nothing >= threshold
  within budget"), every number reproducible from recorded results.
Not in v1: LLVM reproducers, Minotaur/Alive2, STOKE-style search, MCP, other
CPUs, a second target crate (first item for v2).

## 3. Environment facts (observed 11 November 2026)

- Host CPU: AMD Ryzen AI MAX+ 395 (Zen 5), 16 cores / 32 threads, bare metal,
  AVX-512, L2 1 MiB per core, L3 2 x 32 MiB. 122 GiB RAM.
- We run in a shared container. All 32 threads are shared with other agents.
  No reserved cpuset, no CPU quota.
- perf 7.0.14 installed; `perf_event_paranoid=2`; user-space counters work
  (`perf stat -e cycles,instructions,branch-misses` returns `:u` counts).
  Kernel and system-wide events are not available.
- rustc/cargo 1.97.1, LLVM 22.1.6; `llvm-tools` component installed. Not
  installed: llvm-mca, llvm-bolt, cargo-show-asm, cargo-pgo, clang, z3.
- CQLite: workspace v0.18.0, core crate `cqlite-core` (nom parsers), pinned to
  Rust 1.97.1. Release: fat LTO, cgu=1, panic=abort, strip. No target-cpu, no
  PGO. 18 Criterion bench targets, a perf regression gate, 6 fuzz targets,
  proptest. About 25 `unsafe` sites, no explicit SIMD. Checksums and codecs
  come from crates (crc32fast, lz4_flex, snap, flate2, zstd).
- Candidate hot code: `cqlite-core/src/parser/vint.rs`,
  `storage/sstable/reader/parsing/row_decoder/`, `storage/sstable/index_reader/`,
  `storage/sstable/bti/`, `storage/sstable/chunk_decompressor.rs`.
- One Incus VM slot (4 CPU / 12 GiB) exists; whether it exposes PMU counters is
  undocumented.

## 4. Left to the builder (not yet decided)

- how B is linked in exactly (`global_asm!` vs naked functions);
- results-store format (the owner's brief proposed a per-candidate record:
  commit, function, corpus hash, toolchain, target CPU, flags, candidate type,
  correctness evidence and scope, cycles per op for A and B, end-to-end effect,
  interval, search core-hours, regression summary, artifacts);
- whether findings are cached across runs;
- how the CQLite version is pinned;
- calibration suite composition and the recovery-rate target.
