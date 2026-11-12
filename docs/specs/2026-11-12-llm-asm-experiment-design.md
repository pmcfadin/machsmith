# Experiment design: can an LLM hand-tune assembly better than the optimizer?

Status: APPROVED by the owner in conversation, 12 November 2026.
Supersedes, for now, `2026-11-12-machsmith-design-draft.md` (the toolkit
design). The toolkit is redesigned after this experiment, using its results.

Plan that implements this spec: `docs/plans/2026-11-12-llm-asm-experiment/README.md`.

## 1. Why

The owner once worked with programmers who hand-wrote assembly to tune code and
forbade the optimizer. The question, from a conversation with a colleague: could
an LLM do that now? The longer-term goal is a toolset a coding agent uses the
way a Java developer uses PMD (run it, make changes, run it again) to
hyper-optimize Rust. This experiment comes first. It answers the question on
real code and shows what such a tool should look for.

A "no, the compiler wins" result is a valid and useful outcome.

## 2. The question, precisely

On three real CQLite kernels, measured in isolation on this machine, does
assembly written by an LLM agent beat the code rustc produces at its best, by
more than this machine's measurement noise? And where it does, is the win
something a Rust rewrite also gets (the LLM found a better way to write the
function) or not (the LLM out-coded the optimizer)?

## 3. Contestants

For each kernel:

| Name | What it is |
|---|---|
| `A.r1` | CQLite's Rust code, built like CQLite's release profile: opt-level 3, fat LTO, `codegen-units=1`, `panic=abort`, generic x86-64 |
| `A.r2` | `A.r1` plus `-C target-cpu=znver5` |
| `A.r3` | `A.r2` plus profile-guided optimization (PGO) trained on the search inputs. This is "the compiler at its best" |
| `B.asm` | x86-64 assembly written by an LLM agent |
| `B.rust` | a Rust rewrite written by a different LLM agent, built with the `A.r3` recipe (and, in the final exam only, also with the `A.r2` recipe) |

Reference arms, reported but not contestants:

| Name | Purpose |
|---|---|
| `A.r3copy` | a second, separately built copy of `A.r3`. `A.r3copy` vs `A.r3` is the noise-and-layout null |
| `A.reasm` | `A.r3`'s own assembly text pushed through the same assemble-and-link path as `B.asm`, where the kernel allows it. A second null. Not producible for `vint`, whose compiler output calls `memcpy` (section 5.1) |
| `A.inline` | the kernel inlined into its driving loop, as it would be inside CQLite. `A.r3` vs `A.inline` is the "boundary tax" of isolating the kernel |

The headline verdict for a kernel is `B.asm` vs `A.r3`.

## 4. Kernels

Three, each a leaf function behind a C calling-convention boundary. They were
fixed on 12 November 2026 from the preliminary evidence in section 5.1; the
executor does not re-select them.

1. **`vint`**: CQLite's `decode_unsigned`
   (`cqlite-core/src/parser/vint.rs`), the canonical unsigned varint decoder.
   Named by the owner.
2. **`index_framing`**: `parse_big_index_entry_framing`
   (`cqlite-core/src/storage/sstable/index_reader/parse.rs`), which frames one
   BIG-format index entry. This is the index-traversal kernel.
3. **`bti_sparse_find`**: the `Sparse` arm of `BtiNode::find_child`
   (`cqlite-core/src/storage/sstable/bti/node.rs`), a binary search for a child
   by byte in a trie node. A second, differently shaped index kernel.

The owner asked for a varint decoder, one piece of row/cell decoding and one
index-traversal routine "if the profile supports those choices". The profile
does not support a row/cell kernel: **that category is unfilled**, and the
write-up says so (section 4.2).

### 4.1 Admissibility contract

A kernel is admissible only if all of these hold:

- it can be expressed as one `extern "C"` function over integers, raw pointers
  with lengths, and `#[repr(C)]` plain-data structs;
- it allocates nothing and does no I/O; the only calls the compiler's build of
  it may contain are to `memcpy`, `memmove`, `memset`, `memcmp` or `bcmp`;
- it never panics on any input (malformed input returns an error status);
- its Rust source, including helpers it inlines, is at most about 100 lines;
- its inputs can be recorded from real SSTables and replayed.

The C boundary is a thin shim written for this experiment around CQLite's
function, which is copied verbatim. The shim is part of `A` for every rung, so
all contestants implement the same interface.

### 4.2 Why the row/cell category is unfilled

In the preliminary profile of the `read` benchmark, the row-decoding functions
that carry time are `parse_row_data_with_offset_impl` (7.2% of samples),
`parse_clustering_prefix` (2.8%), `parse_row_metadata` (1.9%),
`parse_cell_value_schema_order` (1.5%), `parse_value_from_raw_bytes_reporting`
(1.2%), `parse_complex_column_inner` (1.0%). Each is hundreds of lines, returns
CQLite's `Value` or `Result` types, and allocates. None can be put behind a C
boundary without rewriting it, which would no longer be CQLite's code. The one
small function in the list, `checked_vuint_exact_length` (0.6%), is a comparison
against a one- or two-element list wrapped in an error path that allocates a
message. Task 2 of the plan re-measures these figures with the full method; the
kernel choice stands either way, and `SELECTION.md` records both sets of numbers.

## 5. Step 1: where the time goes

Before the contest, profile CQLite's real release-equivalent build (its
`perfprof` profile: release settings with line tables, not stripped) on three
of its own Criterion benchmarks: `read`, `partition_lookup`, `decode`. Report,
per benchmark, the share of user-space CPU cycles in these buckets: CQLite's
own decoding code, allocator, memory copy, decompression, checksums,
channels and synchronization, everything else.

The share in "CQLite's own decoding code" is the **ceiling**: the most that
perfect kernels could ever speed up that workload. It is reported in the
write-up beside every verdict. It does not gate the experiment: the owner chose
to measure kernels in isolation whatever the ceiling is.

Benchmarks read their data from the **original** repository's
`test-data/datasets` (read-only). The clone's copy is incomplete: the SSTable
files are fetched, not tracked in git.

### 5.1 Preliminary evidence (12 November 2026)

Gathered before the plan was finalized, on this host, from a scratch clone at
the pinned commit built with CQLite's `perfprof` profile. Each benchmark case
ran for 2 seconds under `perf record -e cycles:u`. The numbers are indicative:
short runs, with Criterion's in-process profiler active. Task 2 repeats this
properly.

| Benchmark | CQLite decoding symbols | Allocator | Other libc (mostly memory copy) | Other CQLite | Notes |
|---|---|---|---|---|---|
| `read` | about 23% | about 15% | about 24% | about 25% | attributing inlined code to its source file, CQLite's own decoding lines are only a few percent; `vint.rs` is 0.06% |
| `partition_lookup` | about 54% | 0% | about 17% | 0% | 46% is `IndexReader::lookup_partition`, a hash-map lookup: SipHash is about 32% of all samples. No index parsing is exercised |
| `decode` | about 8% | about 3% | under 1% | about 80% | 40% is `CqlType::parse_with_depth`, which parses type-name strings, not SSTable bytes |

What this means: none of the three kernels is a CQLite hot spot on these
workloads. The experiment still answers the owner's question (can an LLM
out-code the optimizer on a real function), but a win here would move CQLite's
end-to-end speed by well under 1%. This is stated in the write-up.

Other findings that shaped the plan:

- **The compiler's `decode_unsigned` calls `memcpy`.** In a probe build of the
  copied function at every rung, and in CQLite's own fat-LTO benchmark binary,
  the 1-to-8-byte `copy_from_slice` is compiled to a call to the C library's
  `memcpy` through the global offset table. This is the kind of thing the
  experiment exists to find, and it was found by reading the assembly before any
  candidate was written. It is not told to the candidate agents: they receive
  the compiler's assembly and can see it themselves.
- **The local index data is tiny.** All BIG-format `Index.db` files together
  hold 4,629 entries in 112,593 bytes; 151 of 152 files replay cleanly as a
  chain. The measurement protocol therefore has a small-trace mode (section 7).
- **The build recipes work on this host**: forced 64-byte function alignment,
  profile-guided optimization through a loaded library, the `no_std` link of
  Rust candidates, and in-process hardware counters. Details are in the plan's
  "Verified on this host" table.

## 6. Correctness rules

`A.r1` is the reference. All `A` builds must agree with each other first.

A candidate passes only if, for every test input:

- its return value and every output it writes equal the reference's;
- on an error return it leaves the output untouched;
- it never reads or writes outside the bytes it was given (inputs are placed
  flush against inaccessible memory on each side, so any such access crashes);
- it obeys the System V x86-64 calling convention: callee-saved registers
  (`rbx`, `rbp`, `r12`-`r15`) and `rsp` restored, direction flag clear on
  return, `MXCSR` unchanged, upper vector state cleared if it used AVX;
- it terminates within the time limit.

Test inputs:

- **trace inputs**: every call replayed from the recorded real-SSTable inputs;
- **edge cases**: generated per kernel to cover every structural case and
  every truncation point;
- **random inputs**: seeded pseudo-random bytes.

A static lint runs before any candidate code is loaded. It rejects system
calls, calls or jumps out of the kernel, indirect branches, writable or
initializer sections, external symbols, and more than a small fixed amount of
read-only data (so a candidate cannot memorize inputs).

Candidates run in a child process. The parent computes the reference answers
and does the comparison, so a misbehaving candidate cannot corrupt its judge.

Labels: every result is **"tested only"**, with the number of cases. No formal
equivalence is attempted. Nothing from this experiment is suitable to ship.

## 7. Measurement rules

- **Primary metric:** user-space CPU cycles per operation, from hardware
  counters read in-process. Wall-clock nanoseconds, instructions and branch
  misses per operation are always recorded beside it.
- **Inputs:** replayed from real SSTables. Within one sample the inputs do not
  repeat, and where the kernel is used as a chain (the next input starts where
  the last one ended) the chain is preserved.
- **Small-trace mode:** where a kernel's recorded inputs are too few to cut
  into non-repeating samples, a sample is several passes over the whole split
  with the segments in a freshly shuffled order. Results for that kernel are
  labeled "inputs repeat": the branch predictor may learn them, so they describe
  warm-predictor behavior. `index_framing` and `bti_sparse_find` are expected to
  need this mode.
- **Pairing:** all contestants run on the same input chunk within a block, in
  a seeded random order, pinned to one core.
- **Replication unit:** a session, which is a fresh process with address-space
  randomization on, a random environment size, and a shuffled load order. The
  confidence interval is a bootstrap over sessions, not over samples.
- **Entry alignment:** every contestant's entry point is aligned to 64 bytes.
- **Sample hygiene:** a sample is discarded if the thread was preempted, the
  counters were multiplexed, or its instruction count differs from that
  contestant's usual count on that chunk. A block with any discarded sample is
  discarded whole.

### 7.1 The ruler check (null gate)

Before any candidate is judged, and again right before each kernel's final
exam, measure `A.r3copy` vs `A.r3` (and `A.reasm` vs `A.r3` where available)
with the final-exam protocol. The **null band** `b` is the largest distance of
any null confidence bound from 1.0.

- Claim threshold `T = max(5%, 2 × b)`.
- If `b > 5%` on three tries at different times, the ruler is broken: stop and
  report to the owner. Do not run the contest.

### 7.2 Verdicts

With `ratio = candidate cycles per op / baseline cycles per op` and its 95%
interval `[lo, hi]`:

| Verdict | Rule |
|---|---|
| WON | `hi < 1 − T` |
| LOST | `lo > 1 + T` |
| TIED | `lo ≥ 1 − T` and `hi ≤ 1 + T` |
| INCONCLUSIVE | anything else |

## 8. Contest rules

- A fresh agent per kernel and per arm (assembly or Rust): six agents. An
  assembly agent never sees the Rust arm's work, and the reverse.
- Each agent is given: the kernel's contract, CQLite's Rust source for it, the
  compiler's assembly at each rung, and per-rung counter figures on the search
  inputs.
- Budget: **10 counted attempts**. An attempt that fails to build or fails the
  lint is free, up to 30 submissions in total. An attempt that reaches the
  correctness check counts, pass or fail.
- After each attempt the agent sees the correctness verdict (with one failing
  input if it failed) and, if it passed, the timing against each `A` rung.
  These search-phase timings are provisional and are never reported as results.
- The agent picks one best attempt.
- **Final exam, once:** the chosen candidate is checked on the holdout trace
  inputs and on fresh random inputs seeded from the candidate's own hash, then
  measured with the final-exam protocol. Only this result goes in the write-up.
  A candidate that fails the final correctness check is disqualified, with no
  second pick.
- The harness, the inputs and the ledger belong to the orchestrator. Candidate
  agents may write only inside their own candidate directory. The attempt
  command refuses to run if anything else in the repository has changed.
- Every submission, pass or fail, is recorded in an append-only ledger.

## 9. Deliverable

A write-up, `docs/results/2026-11-llm-asm-experiment.md`, containing:

- the answer to the question in section 2, in a few sentences;
- the ceiling figures from section 5;
- for each kernel: the verdict of `B.asm` and `B.rust` against each `A` rung,
  the direct `B.asm` vs `B.rust` comparison, the boundary tax, the null band
  and threshold used, and how many attempts each agent used;
- for each win: what the candidate does differently, with the assembly
  difference and the counter evidence, and a classification of the knowledge it
  used: (a) pure code generation, (b) a fact true in CQLite but invisible to
  the compiler, (c) a property of the workload's data;
- the limitations in section 10, stated plainly;
- what the result suggests the future tool should look for.

Every number in the write-up cites the ledger records it came from.

## 10. Known limitations (to be repeated in the write-up)

- **Isolation.** Kernels are measured behind a function boundary, not inside
  CQLite. A win here does not show CQLite gets faster end to end. The boundary
  tax and the ceiling say how far the result might carry.
- **One machine.** One shared Zen 5 host with no reserved cores. Claims below
  the threshold `T` are not made.
- **Tested, not proven.** Correctness rests on differential testing.
- **Not CQLite's hot spots.** By the profile, the three kernels together are
  a negligible share of CQLite's time on its own benchmarks, and no row/cell
  kernel could be isolated.
- **Holdout by convention.** Holdout inputs come from the same public test
  data, split by a hash of each recorded segment's content, so related inputs
  from one table can fall on both sides. The read-only-data cap and hash-seeded final inputs make memorizing
  them impractical, but agents are kept from them by instruction, not by
  access control.
- **One model, one budget.** The result is "what this agent found in 10
  attempts", not a bound on what is possible.
- **Small data.** CQLite's local test data is about 7.5 MB, so all inputs are
  cache-resident, and two kernels are measured with repeating inputs.

## 11. Out of scope

Swapping code into the real CQLite binary; end-to-end speed-up claims; formal
equivalence; BOLT; other CPUs; Minotaur and Alive2; the reusable toolkit;
changing the CQLite repository; filing the reserved-cores request.

## 12. Council findings carried forward to the toolkit redesign

Recorded here so they are not lost. They do not change this experiment beyond
what sections 3 to 8 already adopt.

- With fat LTO there is no swappable symbol for small kernels; a real-binary
  gate needs a three-arm rebuild (shipped, outlined Rust, outlined candidate).
- The end-to-end gate needs a ceiling precondition and a predicted effect, not
  a flat 1% threshold.
- Search and confirmation must be separate commands, with confirmation counted
  against a false-positive budget.
- A Rust-source arm should be co-equal with the assembly arm.
- Findings should be classified by the knowledge they use.
- Calibration needs planted no-win cases as well as planted slowdowns, and
  cases of realistic size (for example, recovering the known gap between two
  ladder rungs).
- The agent-facing tools need job ids for long builds, bounded assembly dumps,
  a kernel ABI card, and a verdict enum that separates "can't tell" from "no
  difference".
