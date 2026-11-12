# Can an LLM hand-tune assembly better than the optimizer? Results of the machsmith experiment

12 November 2026. Every number below is pasted from `msx report` output or taken from the ledger record named beside it (`experiment/results/ledger.jsonl`). All correctness statements mean "tested only": differential testing against `A.r1`, never a proof. Nothing here is suitable to ship.

## 1. Answer

The threshold was T = 5% (T = max(5%, 2 x null band); the final null bands were 0.042% for `vint`, L000016, and 0.009% for `index_framing`, L000013), and the experiment ran on two kernels, because `bti_sparse_find` recorded 0 calls and was dropped. On `vint`, assembly written by the model beat `A.r3` (the compiler at its best on this machine) with ratio 0.867, interval 0.867–0.868, WON (L000016), and the Rust rewrite got nearly the same win: `B.rust` vs `A.r3` 0.890 WON, and `B.asm` vs `B.rust` 0.975 TIED (L000016). On `index_framing`, assembly beat `A.r3` narrowly: 0.943, interval 0.943–0.944, WON (L000013), a margin of less than one percentage point past T, while the Rust rewrite was TIED with `A.r3` at 0.983 and `B.asm` vs `B.rust` was TIED at 0.960 (L000013), so on that kernel the assembly's edge over the Rust rewrite cannot be claimed. Both arms used one counted attempt each, except the `index_framing` Rust agent, which used two; no candidate failed a correctness check (tested only), and nobody was disqualified (L000007 to L000011, L000013, L000016). These are isolated-kernel results measured with repeating inputs behind a function boundary, on one shared machine, for one model; the kernels are at most 0.09% of CQLite's own profile and `own_decoding` is at most 5.8% of any profiled workload, so no end-to-end CQLite speed-up is claimed. The row/cell category the owner asked for stayed unfilled: no such function meets the admissibility contract.

## 2. What was tested

**Question** (spec section 2): on real CQLite kernels, measured in isolation on this machine, does assembly written by an LLM agent beat the code rustc produces at its best, by more than this machine's measurement noise, and is a win something a Rust rewrite also gets.

**Kernels.** Three were fixed in advance: `vint` (`decode_unsigned`, the canonical unsigned varint decoder), `index_framing` (`parse_big_index_entry_framing`, which frames one BIG-format index entry) and `bti_sparse_find` (the `Sparse` arm of `BtiNode::find_child`). They were chosen for being small, admissible and representative of three decoder shapes (a varint, a nom-framed index entry, a binary search), not for being hot (`experiment/kernels/SELECTION.md`). `bti_sparse_find` was dropped in trace capture: its hook recorded 0 calls in 23 traced benchmark cases, because `find_child` is reached only from CQLite unit tests and the read path uses `find_child_offset`; the plan's drop rule (fewer than 2,000 operations) applied, and no inputs were synthesized. The experiment therefore ran on two kernels. Trace sizes: `vint` search 1,024,899 operations, holdout 157,892 (the holdout is a small trace); `index_framing` search 1,920, holdout 2,631 (both small traces) (`experiment/traces/*/MANIFEST.json`).

**Contestants.**

| name | what it is |
|---|---|
| `A.r1` | CQLite's code built like its release profile (opt-level 3, fat LTO, `codegen-units=1`, `panic=abort`, generic x86-64) |
| `A.r2` | `A.r1` plus `-C target-cpu=znver5` |
| `A.r3` | `A.r2` plus profile-guided optimization trained on the search inputs; "the compiler at its best" |
| `B.asm` | x86-64 assembly written by an LLM agent |
| `B.rust` | a Rust rewrite by a different agent, built with the `A.r3` recipe; `B.rust.r2` is the same source built with the `A.r2` recipe (final exam only) |
| `A.r3copy` | a second build of `A.r3`, byte-identical to it (same sha256); `A.r3copy` vs `A.r3` is the null |
| `A.inline` | the kernel inlined into its driving loop; `A.r3` vs `A.inline` is the boundary tax |

No `A.reasm` arm exists: for `vint` the compiler's output calls `memcpy` through the GOT, and for `index_framing` it references `memcpy` and panic symbols the same way, so neither could be pushed through the assembly path (`experiment/selftest/reasm/README.md`). The spec's second null does not exist for this run.

**Machine and dates.** AMD Ryzen AI MAX+ 395 (Zen 5), Linux 7.0.0-42-generic, governor `powersave`, a shared host with no reserved cores; rustc 1.97.1. All records are dated 12 November 2026 (04:02 to 07:04 UTC; see the provenance block in section 9). CQLite commit `2e5b7a4cf6abe6f51b7677ecb7199e2cab087c0e`.

**Who wrote the candidates.** `claude-sonnet-5-5` (the `model` field of every attempt record, L000007 to L000011), one fresh agent per kernel and arm, four agents. The budget was 10 counted attempts each.

**Measurement.** The primary metric is user-space cycles per operation from in-process hardware counters; the ratio is candidate / baseline, so below 1 the candidate is faster. Each final exam is 30 sessions of 200 blocks, with a bootstrap over sessions for the 95% interval. Entry alignment was normalized: the `A` rungs are built with `-align-all-functions=6` and checked at build time to enter at a 64-byte offset, the assembly candidates declare `.p2align 6`, and the Rust candidates are built with the same flag. Verdicts: WON if the interval's upper bound is below 1 - T, LOST if the lower bound is above 1 + T, TIED if the whole interval lies within 1 +/- T, otherwise INCONCLUSIVE.

## 3. Where CQLite's time goes

Shares of user-space cycle samples on CQLite's own Criterion benchmarks, built with its `perfprof` profile (`perf record -e cycles:u -F 4999`, one pinned core). Rows sum to 100.

| bench | samples | own_decoding | allocator | memcpy | decompression | checksum | sync | other_cqlite | bench_harness | other | source |
|---|---|---|---|---|---|---|---|---|---|---|---|
| read | 191607 | 5.8 | 30.6 | 7.3 | 0.4 | 0.0 | 1.7 | 7.5 | 2.4 | 44.3 | `profile/read.json` |
| partition_lookup | 119876 | 5.6 | 0.1 | 0.0 | 0.0 | 0.0 | 0.0 | 0.0 | 0.2 | 94.2 | `profile/partition_lookup.json` |
| decode | 620706 | 5.5 | 3.4 | 0.1 | 0.0 | 0.0 | 0.0 | 13.6 | 0.3 | 77.1 | `profile/decode.json` |

- `read`: allocation and memory copy dominate; if every decoding kernel were infinitely fast the benchmark would speed up by at most 5.8% of its user-space time.
- `partition_lookup`: the single `own_decoding` entry is `IndexReader::lookup_partition`, a hash-map lookup of an already-loaded index, not index parsing, so the 5.6% ceiling says nothing about the index-framing kernel.
- `decode`: most of the time is parsing CQL type-name strings, not SSTable bytes; the ceiling is at most 5.5%.

**None of the kernels is a CQLite hot spot, and no row/cell kernel could be isolated.** `decode_unsigned` is 0.09% of the `read` samples and 0.04% of the `decode` samples (`profile/read.json`, `profile/decode.json`, `SELECTION.md` section 1). For `index_framing` and `bti_sparse_find` the "below 0.09%" and "below 0.01%" figures in `SELECTION.md` are top-N cutoff bounds (the 15th listed entry), not measurements: the functions were not in the top 15. The row/cell functions that do carry time (`parse_row_data_with_offset_impl`, `parse_clustering_prefix`, `parse_value_from_raw_bytes_reporting` and others) are hundreds of lines, take CQLite's schema or reader types, return `Value` or `Result` and allocate, so none can sit behind a C boundary without being rewritten; the row/cell category is unfilled (`SELECTION.md` sections 2 and 4). Caveats on the ruler: the ceiling is about 5.5 to 5.8% `own_decoding`, samples are attributed to the innermost function after resolving inlining (so inlined helpers land in `other` and `own_decoding` is a lower bound), and the allocator bucket uses an approximate malloc-span heuristic because libc is stripped (`experiment/results/profile/ceiling.md`).

**An observation made before the contest: the compiler turns the byte copy in `decode_unsigned` into a call to `memcpy`.** The Rust source copies the 1-to-8 trailing bytes into an 8-byte stack array (`let mut be = [0u8; 8]`) and reads it back as a big-endian `u64`. In the `A.r3` build of `msx_vint` (`experiment/candidates/vint/BRIEF/A.r3.s`), the array is zeroed, small widths are copied inline, and every other width ends in an indirect call:

```
;     let mut be = [0u8; 8];
   11fc3:      	movq	$0x0, 0x8(%rsp)
   ...
   11fdb:      	cmpq	$0x2, %rax
   11fdf:      	je	0x11fed <msx_vint+0x6d>
   11fe1:      	cmpl	$0x1, %eax
   11fe4:      	jne	0x12024 <msx_vint+0xa4>
   ...
   12024:      	cmpl	$0x6, %eax
   12027:      	jne	0x12037 <msx_vint+0xb7>
   ...
   12040:      	callq	*0x3aa9a(%rip)          # 0x4cae0 <writev+0x4cae0>
```

The disassembler's `writev` label is wrong; `llvm-readobj --relocations` on the `A.r3` library shows the slot at `0x4CAE0` is `R_X86_64_GLOB_DAT memcpy@GLIBC_2.14`. The `A.r1` and `A.r2` listings call `memcpy` through slots `0x4CA50` and `0x4CA10`, also `GLOB_DAT memcpy`, for every multi-byte value; in `A.r3` the profile-guided build inlined the copies for the widths the search data uses most (the ladder tests the trailing-byte counts 2, 1 and 6) and left the call for the rest. On the search split those rest widths (5- and 6-byte encodings) are 817 of 1,024,899 operations, about 0.08% (`experiment/kernels/CARDS/vint.md` section 7). So the call itself is rarely executed here; what the compiler's version pays on every call is the frame and the stack round trip, which section 5 shows.

## 4. Results per kernel

The tables below are `msx report` output, unedited. The ratio is candidate / baseline cycles per op, the interval is the 95% bootstrap over sessions (3 decimals), and the last column is the ledger record the row came from. Percentages in the prose are 100 x (1 - ratio) from the 3-decimal ratio shown, or relative changes between the counter medians in the counters table.

### vint (final L000016)

| comparison | ratio | 95% interval | verdict | ledger id |
|---|---|---|---|---|
| B.asm vs A.r1 | 0.805 | 0.805–0.805 | WON | L000016 |
| B.asm vs A.r2 | 0.818 | 0.818–0.818 | WON | L000016 |
| B.asm vs A.r3 | 0.867 | 0.867–0.868 | WON | L000016 |
| B.rust vs A.r1 | 0.826 | 0.825–0.826 | WON | L000016 |
| B.rust vs A.r2 | 0.839 | 0.839–0.839 | WON | L000016 |
| B.rust vs A.r3 | 0.890 | 0.890–0.890 | WON | L000016 |
| B.rust.r2 vs A.r2 | 0.848 | 0.848–0.849 | WON | L000016 |
| B.asm vs B.rust | 0.975 | 0.974–0.975 | TIED | L000016 |
| A.r2 vs A.r1 | 0.984 | 0.984–0.985 | TIED | L000016 |
| A.r3 vs A.r2 | 0.943 | 0.943–0.943 | WON | L000016 |
| A.r3copy vs A.r3 (null) | 1.00019 | 0.99996–1.00042 | NULL | L000016 |
| A.r3 vs A.inline (boundary tax) | 2.103 | 2.101–2.104 | BOUNDARY | L000016 |

Null band, threshold and measurement conditions, as printed by `msx report`:

Null band 0.00042 (0.042% of 1.0); T = 0.050 (T = max(0.05, 2 x band)). Ratio is candidate / baseline cycles per op; below 1 the candidate is faster. No `A.reasm` arm exists, so the only null is A.r3copy vs A.r3 (identical bytes).

Measured on the holdout split, 30 sessions x 200 blocks, core 5, session reruns 1, inputs repeat: yes (small-trace mode). Rejected sessions: 1.

Counters (medians over the final exam's valid blocks):

| contestant | cycles/op | instructions/op | branch misses/op | ns/op | ledger id |
|---|---|---|---|---|---|
| A.r1 | 12.07 | 49.01 | 0.0462 | 2.355 | L000016 |
| A.r2 | 11.88 | 46.22 | 0.0443 | 2.318 | L000016 |
| A.r3 | 11.20 | 41.60 | 0.0450 | 2.185 | L000016 |
| A.r3copy | 11.20 | 41.60 | 0.0451 | 2.186 | L000016 |
| A.inline | 5.32 | 24.40 | 0.0087 | 1.040 | L000016 |
| B.asm | 9.72 | 26.54 | 0.0392 | 1.896 | L000016 |
| B.rust | 9.96 | 27.72 | 0.0420 | 1.944 | L000016 |
| B.rust.r2 | 10.07 | 27.69 | 0.0411 | 1.966 | L000016 |

- **Attempts.** Each agent used one counted attempt and one submission: `B.asm` L000007, `B.rust` L000008. Neither failed a lint, a build or the correctness check (tested only; 8,166,128 cases in the search check, 40,698,122 in the final check, L000016).
- **Small-trace mode.** The final exam ran on the holdout split, which for `vint` is a small trace (157,892 operations): inputs repeat (`repeat_r` 1, so each sample is one pass over the whole split with the segments shuffled; L000016). The branch predictor and the caches can learn the data across blocks, so these are warm-predictor numbers. The search phase for `vint` used non-repeating inputs; the final exam does not.
- **Boundary tax.** Isolating this kernel behind a call costs 110%: `A.r3` takes 2.103 times the cycles per op of `A.inline` (11.20 against 5.32 cycles/op, L000016). The kernel is only a few cycles of work per call (5.32 cycles/op inlined), so a per-call cost that is invisible for a large function dominates here. I did not isolate the cause (a plausible contributor is the dependent chain through the `out` store and the returned length, which an inlined loop can keep in registers); the measurement says only that this kernel's isolated numbers cannot be carried into CQLite unchanged.
- **The ladder.** `A.r2` vs `A.r1` is 0.984, TIED: `-C target-cpu=znver5` alone bought nothing the threshold can see. `A.r3` vs `A.r2` is 0.943, WON: profile-guided optimization was worth about 5.7% (L000016). The `A.r3` build trained on the search inputs also drives the `_chain` symbols, so `A.inline` is PGO-trained too.
- **Rust at the weaker recipe.** `B.rust.r2` vs `A.r2` is 0.848, WON (L000016): the Rust rewrite keeps its win when built without PGO.

### index_framing (final L000013)

| comparison | ratio | 95% interval | verdict | ledger id |
|---|---|---|---|---|
| B.asm vs A.r1 | 0.813 | 0.813–0.814 | WON | L000013 |
| B.asm vs A.r2 | 0.874 | 0.874–0.875 | WON | L000013 |
| B.asm vs A.r3 | 0.943 | 0.943–0.944 | WON | L000013 |
| B.rust vs A.r1 | 0.848 | 0.847–0.848 | WON | L000013 |
| B.rust vs A.r2 | 0.911 | 0.911–0.911 | WON | L000013 |
| B.rust vs A.r3 | 0.983 | 0.983–0.983 | TIED | L000013 |
| B.rust.r2 vs A.r2 | 0.911 | 0.911–0.911 | WON | L000013 |
| B.asm vs B.rust | 0.960 | 0.960–0.960 | TIED | L000013 |
| A.r2 vs A.r1 | 0.930 | 0.930–0.930 | WON | L000013 |
| A.r3 vs A.r2 | 0.927 | 0.927–0.927 | WON | L000013 |
| A.r3copy vs A.r3 (null) | 1.00001 | 0.99994–1.00009 | NULL | L000013 |
| A.r3 vs A.inline (boundary tax) | 0.939 | 0.938–0.939 | BOUNDARY | L000013 |

Null band, threshold and measurement conditions, as printed by `msx report`:

Null band 0.00009 (0.009% of 1.0); T = 0.050 (T = max(0.05, 2 x band)). Ratio is candidate / baseline cycles per op; below 1 the candidate is faster. No `A.reasm` arm exists, so the only null is A.r3copy vs A.r3 (identical bytes).

Measured on the holdout split, 30 sessions x 200 blocks, core 15, session reruns 0, inputs repeat: yes (small-trace mode). Rejected sessions: 0.

Counters (medians over the final exam's valid blocks):

| contestant | cycles/op | instructions/op | branch misses/op | ns/op | ledger id |
|---|---|---|---|---|---|
| A.r1 | 29.35 | 87.75 | 0.0132 | 5.911 | L000013 |
| A.r2 | 27.30 | 81.75 | 0.0132 | 5.498 | L000013 |
| A.r3 | 25.31 | 83.75 | 0.0147 | 5.099 | L000013 |
| A.r3copy | 25.31 | 83.75 | 0.0147 | 5.099 | L000013 |
| A.inline | 26.97 | 66.48 | 0.0135 | 5.430 | L000013 |
| B.asm | 23.87 | 52.45 | 0.0200 | 4.808 | L000013 |
| B.rust | 24.88 | 48.60 | 0.0132 | 5.009 | L000013 |
| B.rust.r2 | 24.88 | 48.60 | 0.0132 | 5.009 | L000013 |

- **Attempts.** The assembly agent used one counted attempt (L000009). The Rust agent used two (L000010, L000011) and chose `attempt-01`; `attempt-02` is in the attempts history below. None failed a lint, a build or the correctness check (tested only; 4,759,932 cases in each search check, 40,762,802 in each final check, L000013).
- **Small-trace mode.** Both splits are small traces (1,920 and 2,631 operations). The final exam repeats the whole split 20 times per sample (`repeat_r` 20, L000013), so the same few thousand inputs are replayed. The branch predictor can memorize them: branch misses are 0.0147 per op for `A.r3` and 0.0200 for `B.asm` (L000013), tiny in both cases, and the cycle counts describe warm-predictor, cache-resident behaviour. They say little about index entries seen once.
- **Boundary tax.** The ratio of `A.r3` to `A.inline` is 0.939 (L000013): in this harness the isolated call is 6.1% cheaper than the inlined chain, so the tax is negative here, even though `A.inline` retires fewer instructions (66.48 against 83.75 per op). I did not investigate why; it is a reminder that code placement and the loop around a kernel move cycles by amounts comparable to the effects being claimed.
- **The ladder.** `A.r2` vs `A.r1` is 0.930, WON, and `A.r3` vs `A.r2` is 0.927, WON (L000013): on this kernel both `target-cpu` and PGO were worth more than 7%.
- **Rust at the weaker recipe.** `B.rust.r2` vs `A.r2` is 0.911, WON, the same as `B.rust` vs `A.r2` (L000013). `B.rust.r2` and `B.rust` are separate builds, but the instruction sequences of `msx_index_framing` in the two libraries are identical once addresses are normalized (a check I ran on the build outputs, not a ledger fact).
- **Null.** `A.r3copy` vs `A.r3` is a null of identical bytes: it measures noise at one code placement and cannot reveal layout sensitivity between different binaries.

### Attempts history

Search-phase ratios appear only here. They come from 6-session screening runs (60 blocks) and are provisional, not results. `counted` is `free` for a submission that failed lint or build and cost nothing. The `vint` search phase used non-repeating inputs, so its numbers are not comparable with the final exam.

**index_framing / asm**

| submission | counted | verdict | provisional (search phase) ratio vs A.r3, or failure kind | ledger id |
|---|---|---|---|---|
| 1 | 1 | PASS | 0.945 (0.944–0.947) | L000009 |

**index_framing / rust**

| submission | counted | verdict | provisional (search phase) ratio vs A.r3, or failure kind | ledger id |
|---|---|---|---|---|
| 1 | 1 | PASS | 0.984 (0.983–0.985) | L000010 |
| 2 | 2 | PASS | 1.058 (1.057–1.059) | L000011 |

**vint / asm**

| submission | counted | verdict | provisional (search phase) ratio vs A.r3, or failure kind | ledger id |
|---|---|---|---|---|
| 1 | 1 | PASS | 0.833 (0.832–0.833) | L000007 |

**vint / rust**

| submission | counted | verdict | provisional (search phase) ratio vs A.r3, or failure kind | ledger id |
|---|---|---|---|---|
| 1 | 1 | PASS | 0.856 (0.855–0.856) | L000008 |

## 5. What the winners did

Three comparisons against `A.r3` were WON: `vint` `B.asm` (L000016), `vint` `B.rust` (L000016) and `index_framing` `B.asm` (L000013). The classification below comes from reading the code beside the compiler's listing and the counters; no ablation was run, so a "mostly" is a judgement, not a measurement.

### vint, B.asm: ratio 0.867, WON (L000016)

`A.r3`'s listing (`BRIEF/A.r3.s`) spends the first instructions of every non-empty call on `pushq %r15; pushq %r14; pushq %rbx; subq $0x10, %rsp`, computes the width with `lzcnt` before it tests for the one-byte case, and for wider values zeroes a stack array, copies the trailing bytes in with width-specific moves, then reloads the array with `movbeq`. The candidate (`experiment/candidates/vint/asm/attempt-01.s`) starts with the case that is 81% of the inputs and leaves immediately:

```
msx_vint:
        testq   %rsi, %rsi
        je      .Lzero
        movzbl  (%rdi), %eax
        testb   %al, %al
        js      .Lmulti
        movq    %rax, (%rdx)
        movl    $1, %eax
        ret
```

For wider values, when at least 8 bytes are readable it does one `movbe` load straight from the input, a variable shift and a `bzhi` that drops the marker bits (no stack array, no width ladder); with fewer than 8 bytes it falls back to a byte loop.

Counters against `A.r3` (L000016): instructions per op 26.54 against 41.60 (-36.2%), branch misses per op 0.0392 against 0.0450 (-13.0%), cycles per op 9.72 against 11.20 (-13.2%).

Classification: **(a) pure code generation, with a minor (c) element.** The contract is unchanged, and the gain is most of the instruction count: no frame on the common path and no stack round trip. The one-byte-first layout uses a property of the workload's data (81.11% of search operations are one byte, `CARDS/vint.md`), but `A.r3` is already profile-guided on the same data and branches early for the one-byte case after its `lzcnt`, so the data fact is not what separates the candidate from the compiler; I cannot say how much of the gain it carries. The `movbe` load relies only on the length check the code makes itself, not on a CQLite fact the compiler cannot see, so this is not (b).

### vint, B.rust: ratio 0.890, WON (L000016)

The Rust candidate (`experiment/candidates/vint/rust/attempt-01.rs`) has the same shape: an empty-input and one-byte fast path with no frame, and the rest in a `#[cold] #[inline(never)]` function that reads overlapping big-endian `u16` or `u32` halves instead of copying into a buffer, so there is no `memcpy` and no width ladder. Counters against `A.r3` (L000016): instructions per op 27.72 against 41.60 (-33.4%), branch misses per op 0.0420 against 0.0450 (-6.6%), cycles per op 9.96 against 11.20 (-11.0%).

Classification: **(a) pure code generation, with the same minor (c) element.** **`B.rust` captured nearly the same win**: `B.asm` vs `B.rust` is 0.975, TIED (L000016), so the assembly's further 2.5% is below T and is not claimed. For `vint` the win is mostly about how the function is written (a frame-less fast path, no buffer round trip) and is reachable from Rust source.

### index_framing, B.asm: ratio 0.943, WON (L000013)

`A.r3`'s listing pushes five registers and subtracts 0x10 from `%rsp` on entry, keeps the 8-byte scratch buffer and the `memcpy` call for wide second varints, and carries the slice-index and `split_at` panic call sites (`BRIEF/A.r3.s`; `SELECTION.md` section 5 says both are unreachable but the compiler did not fold them away). The candidate (`experiment/candidates/index_framing/asm/attempt-01.s`) is straight-line: bounds check, a `movbe` for the key length, one byte load per varint with a sign test so single-byte varints take no further work, and `lzcnt` plus a byte loop only for multi-byte ones. It uses no stack and no callee-saved registers.

Counters against `A.r3` (L000013): instructions per op 52.45 against 83.75 (-37.4%), cycles per op 23.87 against 25.31 (-5.7%), but branch misses per op 0.0200 against 0.0147 (+35.5%), in absolute terms tiny. Instruction count fell by more than a third and cycles by far less, which is consistent with a loop bound by the dependent chain of loads (key length, then key bytes, then the first varint byte, then the second); the candidate's own notes say the same, and I did not test it.

Classification: **(a) pure code generation, with a (c) element.** The contract is unchanged and most of the gain is instructions. The handling of the second varint as a one-byte case first uses the data fact that at least 99% of search entries have a promoted-index length of 0 (`CARDS/index_framing.md` section 7), a property of the workload's data. Neither (b) nor a separate (c) gain was isolated. The win is just past the threshold (hi 0.944 against 0.95).

**Did `B.rust` capture it? No.** The Rust rewrite (`experiment/candidates/index_framing/rust/attempt-01.rs`) has the same structure (position checks only, a one-byte second-varint fast path, a cold out-of-line wide tail) and retires fewer instructions than the assembly (48.60 against 52.45 per op) with fewer branch misses (0.0132), but its cycle ratio is 0.983, TIED against `A.r3` (L000013). Fewer instructions bought only 1.7% fewer cycles (24.88 against 25.31). `B.asm` vs `B.rust` is 0.960, TIED, so the assembly's advantage over the Rust rewrite is itself below T: the data show a narrow win for assembly over the compiler and no demonstrable win over the Rust rewrite.

## 6. What failed

- **Disqualifications:** none. All four finalists were entered (L000013, L000016).
- **Correctness failures:** none, across all five submissions (L000007 to L000011) and both final checks (tested only). Lint failures: none. Build failures: none. By kind: PASS 5, everything else 0. The `index_framing` Rust agent's second submission (L000011) passed correctness but screened slower than `A.r3` (see the attempts history), and the agent did not pick it.
- **`vint` final exam, four tries (incidents).** Three runs ended in `NOISE_GATE_FAILED` with no verdicts: L000012 (core 5, 83 of 200 blocks discarded), L000014 (core 5, 200 of 200) and L000015 (core 2, 150 of 200). Reruns were allowed only after a noise-gate failure and the picks could not change. The cause was another agent's bursty parallel cqlite builds on the shared host, which has no cpuset isolation; the first two tries started without a recorded host state, and tries 3 and 4 started only after a quiet-host gate (no `rustc` or `cargo` process and 1-minute load average under 2.0, held ten minutes; `.superpowers/sdd/README/task-13-report.md`). Try 3 (L000015) still failed on a gate-quiet machine; try 4 (L000016) was OK and is the result reported. Each failed try produced no verdicts, so no result was chosen among tries; the reader should still know the tally. `index_framing` ran once (L000013).
- **Earlier selftest records.** L000001 (a `selftest` with `pass` false: `vint/control_slow.s` hit `NOISE_GATE_FAILED`) and L000002 were produced by an earlier build of the tool (sha256 `d6baaec5...`); L000003 and L000004 were produced by the frozen tool (`9007b7ac...`), and all 22 selftest files passed in both.
- **`incidents.md`:** `experiment/results/incidents.md` does not exist, and `msx report` prints "no incidents file". The incidents are the ones listed here and in the report's incidents block.
- **Dirty-tree flag.** `git_dirty` is true in L000013, L000014, L000015 and L000016 and in the null record L000006. The ledger and the result files were uncommitted while the runs ran; the tree was otherwise clean.

## 7. Limitations

1. **Isolation.** Kernels were measured behind a C function boundary, not inside CQLite. The boundary tax shows how far that matters: 110% for `vint` (2.103, L000016) and a negative 6.1% for `index_framing` (0.939, L000013). The ceiling (at most 5.5 to 5.8% `own_decoding`) and the kernels' 0.09% shares say a win here would move CQLite by well under 1%. No end-to-end speed-up is claimed.
2. **One machine.** One shared Zen 5 host with no reserved cores. The measured null bands were 0.042% (`vint`, L000016) and 0.009% (`index_framing`, L000013), so T was 5% in both cases (the floor); claims smaller than T = 5% are not made. A.r3copy is a byte-identical copy of A.r3, so its null measures layout and placement noise at one placement, not code differences, and no `A.reasm` null exists. The measurement core was chosen automatically per run (core 5 for `vint`, core 15 for `index_framing`); entry alignment was normalized (64-byte function alignment, section 2).
3. **Tested, not proven.** Correctness is differential testing against `A.r1`: 40,698,122 cases for `vint` and 40,762,802 for `index_framing` in the final checks, from replayed traces, edge cases and seeded random inputs. No formal equivalence was attempted.
4. **Not CQLite's hot spots.** By the profile the kernels are a negligible share of CQLite's time (0.09% and 0.04% for `decode_unsigned`), `bti_sparse_find` was never reached on the read path, and no row/cell kernel could be isolated.
5. **Holdout by convention.** Holdout inputs come from the same public test data, split by a hash of each recorded segment, so related inputs from one table can fall on both sides. For `vint` the search and holdout splits come from overlapping datasets. For `index_framing` the holdout (2,631 operations) is larger than the search split (1,920). The read-only-data cap and the hash-seeded final inputs make memorizing impractical, but agents were kept from the holdout by instruction, not by access control.
6. **One model, one budget.** The result is what `claude-sonnet-5-5` found in one or two counted attempts, not in the ten allowed: three of four agents stopped after one counted attempt, and the fourth used two. It is not a bound on what is possible, and a stronger search could find more or find that the wins shrink.
7. **Small data.** CQLite's local test data is about 7.5 MB, so every input is cache-resident, and both kernels were measured with repeating inputs in the final exam (`inputs_repeat` true, L000013 and L000016), so the branch predictor may have learned them.

Found during the run:

- **The Rust rewrite is not CQLite's code.** `B.rust` rewrites each kernel in Rust, so a `B.rust` win says what a rewrite can reach, not that CQLite's source can be tuned in place.
- **Counters are medians, ratios are session bootstraps.** The counters table is the median over valid blocks, while the ratio is the geometric mean of per-session medians with a bootstrap, so the two are consistent but not identical.
- **Discard-cause breakdowns are not reported.** The tool counts discarded blocks and rejected sessions but does not report why (preemption, multiplexing, instruction-count mismatch); this is a known gap.
- **PGO also trains the `_chain` symbols,** so `A.inline` is PGO-trained, and the boundary tax compares two PGO-trained builds.
- **`vint` final exam took four tries** (section 6).
- **The ceiling ruler's malloc-span heuristic is approximate.**

## 8. What this suggests for the tool

Drawn only from what was observed here:

- **Flag variable-length copies into a zero-initialised stack array** (`let mut be = [0u8; 8]; be[8 - n..].copy_from_slice(...)`, then `from_be_bytes`). The compiler emitted a `memcpy` call through the GOT in every rung (section 3) and both `vint` winners removed the buffer (overlapping loads, or one `movbe`).
- **Flag functions whose common case needs no saved registers but whose entry pushes them anyway.** `A.r3`'s `msx_vint` pays `push`/`pop` and `lzcnt` on every call while 81% of the inputs are one byte; both winners put a frame-less fast path first. For Rust the tool can suggest `#[cold] #[inline(never)]` on the rare arm.
- **Flag panic and slice-index call sites the compiler could not prove away** in a small leaf function (the `split_at` and slice-range sites in `index_framing`), since they keep the frame and the callee-saved pushes alive.
- **Do not gate on instruction count.** Instructions per op fell by 33.4 to 42.0% for the four candidates and cycles fell by 1.7 to 13.2% (L000013, L000016). On `index_framing`, `B.rust` retired the fewest instructions and was TIED. The tool should decide on cycles with an interval and show instructions only as explanation.
- **Report the inlined baseline beside every isolated number.** The boundary tax ranged from 2.103 to 0.939; an isolated win of 5.7% is smaller than the effect of the loop around the kernel on either kernel.
- **Say when inputs repeat.** Both final exams repeated their inputs. Branch misses moved the wrong way for the asm candidate on `index_framing` (+35.5%) while cycles improved; a tool that cannot tell whether the predictor learned the data will misread such changes.
- **Try the cheap compiler levers first and report them as a ladder.** `target-cpu` was worth nothing on `vint` (0.984, TIED) and 7.0% on `index_framing` (0.930, WON); PGO was worth 5.7% and 7.3% (L000016, L000013). A hand-tuned candidate should be compared with the top rung, not the bottom one: `B.asm` on `vint` is 0.805 against `A.r1` and 0.867 against `A.r3`.
- **Pair the Rust arm with every assembly arm.** It showed that the `vint` win is reachable from Rust (0.890 WON) and that the `index_framing` win is not demonstrably reachable (0.983 TIED), which is the question the tool should answer for each finding.
- **Which harness pieces proved their worth.** (1) The ruler: the null bands (0.042% and 0.009%) were at least 100 times smaller than T, and the controls recovered known gaps, with `control_slow.s` LOST at 7.548 on `vint` and 3.717 on `index_framing`, and `control_copy.rs` TIED at 0.995 (L000004). (2) The noise gate rejected three real final runs on a busy host (L000012, L000014, L000015) rather than issue verdicts from them. (3) The sabotage cases (`overread1.s`, `underread1.s`, `unsafe_overread.rs`, `clobber_rbx.s`, `wrong_fe.s`, `wrong_long.s`, `writes_on_error.s`, `loop_ff.s`, the lint cases and the `index_framing` `wrong.s` and `overread.s`) were exercised only by selftest (L000003, L000004): no real candidate failed a lint, build or correctness check, so none of them caught a real failure in this run.

## 9. Reproducing this

From the repository root; `experiment/env.sh` keeps all builds under `work/` and never uses the shared target directory.

```bash
. experiment/env.sh
export RUSTUP_TOOLCHAIN=1.97.1
# ceiling: see experiment/results/profile/ceiling.md ("Commands")
# traces: see experiment/trace-capture/README.md
msx_cargo tool build --release -p msx-harness            # the frozen tool: work/target/tool/release/msx
MSX=work/target/tool/release/msx
$MSX rungs build                                         # A.r1, A.r2, A.r3, A.r3copy
$MSX selftest                                            # sabotage and control files
$MSX null --kernel vint; $MSX null --kernel index_framing
# per candidate (MSX_AGENT_ID and MSX_MODEL must be set):
$MSX attempt --kernel vint --arm asm --file experiment/candidates/vint/asm/attempt-01.s
$MSX final --kernel vint                                 # once per kernel; --rerun-reason TEXT only after NOISE_GATE_FAILED
$MSX final --kernel index_framing
# the report (separate build, only reads files):
msx_cargo report build --release -p msx-harness
work/target/report/release/msx report
```

The ledger holds records L000001 to L000016; the final exams are L000013 (`index_framing`) and L000016 (`vint`). Provenance block from `msx report`:

- Ledger id range: L000001 to L000016 (16 records).
- Dates: 2026-11-12T04:02:02Z to 2026-11-12T07:04:17Z.
- Tool sha256 d6baaec5585ac9073857b8eff557bf7ed64c9eef739c8aa7a9716baef04dd566: 2 records (L000001 to L000002).
- Tool sha256 9007b7ac62f047885e85379961804a3ea26c5e26fa0f7462a1cbc6639e9f14bd: 14 records (L000003 to L000016).
- This report was produced by a separate build of the tool, sha256 4748a57b4065a30a482571016d6270304069b5271e0be888dc06925b30269ef1 (it only reads files).
- A hashes (16 records, L000001 to L000016): A.r1 a07c922f88cf05fca4065147724a01f6ef819f903f190781f509bc1a6f2ce889, A.r2 eeb71d93fb0a177c65a170d16b45338ca89b49143b5e51c54b477b0353925564, A.r3 bd4992937b1ccadcb8488770c05d2b8bcadead7ddb678a50eadc328140284fd7, A.r3copy bd4992937b1ccadcb8488770c05d2b8bcadead7ddb678a50eadc328140284fd7.
- CQLite commit: 2e5b7a4cf6abe6f51b7677ecb7199e2cab087c0e.
- Host: AMD RYZEN AI MAX+ 395 w/ Radeon 8060S, Linux 7.0.0-42-generic, governor powersave.
- Records written with `git_dirty` true: L000001, L000002, L000006, L000007, L000008, L000009, L000010, L000011, L000013, L000014, L000015, L000016.
