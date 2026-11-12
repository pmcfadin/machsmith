# Phase 1: profile, kernels, traces (Tasks 1-4)

Read `README.md` in this directory first: it holds the global constraints, file map, shared interfaces and constants every task assumes.

All paths are relative to `/home/dev/projects/machsmith` unless absolute.

---

### Task 1: Repository, workspace skeleton, environment guard

**Files:**
- Create: `.gitignore`, `README.md`
- Create: `experiment/rust-toolchain.toml`, `experiment/Cargo.toml`, `experiment/env.sh`
- Create: `experiment/kernels/Cargo.toml`, `experiment/kernels/src/lib.rs` (empty crate, `crate-type = ["cdylib", "rlib"]`, package `msx-kernels`)
- Create: `experiment/harness/Cargo.toml`, `experiment/harness/src/lib.rs`, `experiment/harness/src/bin/msx.rs` (package `msx-harness`, binary `msx`)
- Test: `experiment/tests/env_guard.sh`

**Interfaces:**
- Produces: `experiment/env.sh`, sourced by every script in this plan. It exports:
  - `MSX_ROOT=/home/dev/projects/machsmith`
  - `MSX_WORK=$MSX_ROOT/work`
  - `MSX_LLVM_BIN=$(rustc --print sysroot)/lib/rustlib/x86_64-unknown-linux-gnu/bin`
  - a shell function `msx_cargo <target-name> <cargo args…>` that runs cargo with `CARGO_TARGET_DIR=$MSX_WORK/target/<target-name>`
  - a shell function `msx_require_env` that exits 1 with a message if `CARGO_TARGET_DIR` is set and does not start with `$MSX_WORK/`, or if `gcc`, `perf`, `python3`, `$MSX_LLVM_BIN/llvm-profdata`, `$MSX_LLVM_BIN/llvm-objdump`, `$MSX_LLVM_BIN/llvm-nm` or `$MSX_LLVM_BIN/llvm-readobj` is missing
  - `unset CARGO_TARGET_DIR` at the top, so the shared value inherited from the login environment never leaks into a direct `cargo` call

- [ ] **Step 1: Initialize the repository**

```bash
cd /home/dev/projects/machsmith && git init -b main
```

`.gitignore` contains `work/` and `**/target/`. `README.md` says in three or four sentences what the repo is and points at `docs/specs/` and `docs/plans/`.

- [ ] **Step 2: Write the failing guard test**

`experiment/tests/env_guard.sh` asserts three things and exits nonzero on the first failure:

```bash
# 1. sourcing env.sh clears the shared target dir
( export CARGO_TARGET_DIR=/home/dev/.cache-dev/cargo-target; . experiment/env.sh; [ -z "${CARGO_TARGET_DIR:-}" ] )
# 2. msx_require_env rejects a target dir outside work/
( . experiment/env.sh; export CARGO_TARGET_DIR=/home/dev/.cache-dev/cargo-target; ! msx_require_env 2>/dev/null )
# 3. msx_cargo builds into work/target/<name>
( . experiment/env.sh; msx_cargo guardtest build -q --manifest-path experiment/Cargo.toml -p msx-harness \
    && [ -d work/target/guardtest/debug ] )
```

- [ ] **Step 3: Run it to see it fail**

Run: `bash experiment/tests/env_guard.sh; echo exit=$?`
Expected: `exit=1` (no `env.sh` yet).

- [ ] **Step 4: Write `env.sh`, the toolchain file and the workspace**

`experiment/rust-toolchain.toml`: channel `1.97.1`, no extra targets. `experiment/Cargo.toml`: workspace with members `kernels`, `harness`, resolver 2, and

```toml
[profile.release]
opt-level = 3
lto = "fat"
codegen-units = 1
panic = "abort"
debug = 1          # line tables only, so llvm-objdump -S and perf can map code to source
strip = "none"
```

This mirrors CQLite's `[profile.release]` except for `debug` and `strip`, which do not change generated code.

Harness dependencies: `libc`, `serde` (derive), `serde_json`, `sha2`, `rand_chacha`, `rand_core`. Nothing else without a reason written in the commit message.

- [ ] **Step 5: Run the guard test and confirm crates.io is reachable**

Run: `bash experiment/tests/env_guard.sh; echo exit=$?`
Expected: `exit=0`. If cargo cannot download crates, stop and report.

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "chore: repository skeleton and environment guard"
```

---

### Task 2: Clone CQLite, profile it, ceiling report

**Files:**
- Create: `tools/profile-cqlite.sh`, `tools/ceiling.py`, `tools/test_ceiling.py`
- Create: `experiment/results/profile/{read,partition_lookup,decode}.json`, `experiment/results/profile/ceiling.md`

**Interfaces:**
- Produces: `work/cqlite/` (clone at the pinned commit), used by Tasks 3 and 4.
- Produces: `experiment/results/profile/<bench>.json` with this shape, used by Task 3 and Task 14:

```json
{
  "bench": "read",
  "cqlite_commit": "2e5b7a4c…",
  "samples": 123456,
  "buckets": { "own_decoding": 0.0, "allocator": 0.0, "memcpy": 0.0, "decompression": 0.0,
               "checksum": 0.0, "sync": 0.0, "other_cqlite": 0.0, "bench_harness": 0.0, "other": 0.0 },
  "top_functions": [ { "function": "cqlite_core::parser::vint::decode_unsigned", "source_file": "…/vint.rs",
                       "bucket": "own_decoding", "share": 0.0, "physical_symbols": ["…"] } ]
}
```

`buckets` are fractions of user-space cycle samples and sum to 1. `top_functions` has the top 40 by share. `function` is the innermost function at the sampled address **after resolving inlining**; `physical_symbols` are the surviving symbols it was found inside.

- [ ] **Step 1: Clone at the pinned commit**

```bash
. experiment/env.sh
git clone --no-hardlinks /home/dev/projects/cqlite "$MSX_WORK/cqlite"
git -C "$MSX_WORK/cqlite" checkout --detach 2e5b7a4cf6abe6f51b7677ecb7199e2cab087c0e
```

The clone is disposable and gitignored. Every bench run sets `CQLITE_DATASETS_ROOT=/home/dev/projects/cqlite/test-data/datasets` (read-only use of the original). Do not use the clone's own `test-data/datasets`: it lacks the fetched SSTables, and against it `partition_lookup` panics on a missing `Index.db` and `read` lists two cases instead of nine.

- [ ] **Step 2: Write the failing classifier test**

`tools/test_ceiling.py` (plain `unittest`) asserts `ceiling.classify(function, dso)` returns:

| function, dso | bucket |
|---|---|
| `cqlite_core::parser::vint::decode_unsigned`, bench binary | `own_decoding` |
| `cqlite_core::storage::sstable::reader::parsing::row_decoder::row_framing::…`, bench | `own_decoding` |
| `cqlite_core::storage::sstable::index_reader::parse::parse_big_index_entry`, bench | `own_decoding` |
| `cqlite_core::storage::sstable::bti::node::BtiNode::find_child`, bench | `own_decoding` |
| `cqlite_core::storage::sstable::chunk_decompressor::…`, bench | `other_cqlite` |
| `lz4_flex::block::decompress::…`, bench | `decompression` |
| `ZSTD_decompressBlock_internal`, bench | `decompression` |
| `crc32fast::…`, bench | `checksum` |
| `malloc`, `libc.so.6` | `allocator` |
| `__rust_alloc`, bench | `allocator` |
| `__memmove_avx512_unaligned_erms`, `libc.so.6` | `memcpy` |
| `crossbeam_channel::…`, bench | `sync` |
| `std::sync::mpmc::…`, bench | `sync` |
| `criterion::…`, bench | `bench_harness` |
| `rustc_demangle::v0::Printer::print_path`, bench | `bench_harness` |
| `pprof::…`, bench | `bench_harness` |
| `_Unwind_Find_FDE`, `libgcc_s.so.1` | `bench_harness` |
| `core::fmt::write`, bench | `other` |

`own_decoding` is exactly: `cqlite_core::parser::`, and `cqlite_core::storage::sstable::` followed by `reader::parsing`, `index_reader`, `bti`, `summary_reader` or `row_cell_state_machine`. `bench_harness` also takes everything in `libgcc_s.so.1` and any function matching `rustc_demangle`, `pprof`, `backtrace`, `gimli` or `addr2line`: with `--profile-time`, Criterion runs an in-process sampling profiler whose own cost (a few percent) must not be counted as CQLite's.

Run: `python3 tools/test_ceiling.py` — expected: fails (module missing).

- [ ] **Step 3: Write `tools/profile-cqlite.sh` and `tools/ceiling.py`**

`profile-cqlite.sh <bench>`:
1. builds the bench in the clone into `work/target/cqlite-perfprof` with
   `cargo bench -p cqlite-core --profile perfprof --features cli-helpers,bench-internals --bench <bench> --no-run`
   (release code generation, line tables, unstripped; about two minutes for the first bench). The binary is `work/target/cqlite-perfprof/perfprof/deps/<bench>-<hash>`;
2. runs it as `<binary> --bench --profile-time 4` under `perf record -e cycles:u -F 4999`, pinned with `taskset` to one core. `--profile-time 4` samples **each case** for 4 seconds: `read` has 9 cases, `partition_lookup` 6, `decode` 31;
3. writes `perf script` output with at least `ip`, `sym`, `symoff`, `dso` to `work/profile/<bench>.script`.

`ceiling.py <bench> <perf-script-file> <bench-binary>`:
- for samples in the bench binary, resolves each address to its inline chain with `addr2line -i -f -C -e <binary>` (batch all unique addresses in one invocation) and takes the innermost function and its source file;
- for samples in other objects, uses the symbol name `perf` reported;
- classifies, aggregates and writes the JSON above.

If `addr2line` cannot map addresses because the binary is position-independent, compute file addresses as `llvm-nm` symbol address plus `symoff`.

- [ ] **Step 4: Run the classifier test**

Run: `python3 tools/test_ceiling.py` — expected: `OK`.

- [ ] **Step 5: Profile the three benches**

```bash
for b in read partition_lookup decode; do bash tools/profile-cqlite.sh "$b"; done
```

Expected: three JSON files, each with `samples` ≥ 50,000.

Sanity check against the preliminary figures in spec section 5.1 (2-second runs, bucketed by surviving symbol rather than by inlined function): `read` had about 15% allocator and about 24% other libc; `partition_lookup` was dominated by `IndexReader::lookup_partition` with SipHash around a third of samples; `decode` was dominated by `CqlType::parse_with_depth`. Your `own_decoding` figures will be **lower** than the preliminary ones because inlined standard-library code is now attributed to its own function. If a bucket differs from the preliminary figure by more than a factor of two in the other direction, check the classifier before believing it.

- [ ] **Step 6: Write `experiment/results/profile/ceiling.md`**

One table: rows are the three benches, columns are the nine buckets as percentages. Below it, for each bench, the top 15 `own_decoding` functions with their share. State the host, date, CQLite commit and the exact commands. State in one sentence per bench what the ceiling means ("if every decoding kernel were infinitely fast, `read` would speed up by at most X%").

Also state, per bench, what it exercises, because two of the names mislead: `partition_lookup` measures a hash-map lookup of an already-loaded index, not index parsing; `decode` spends most of its time parsing CQL type-name strings.

This is a report, not a gate. Continue whatever the numbers are.

- [ ] **Step 7: Commit**

```bash
git add tools experiment/results/profile && git commit -m "feat: CQLite profile and ceiling report"
```

---

### Task 3: `msx-kernels` crate for the three fixed kernels, kernel cards

**Files:**
- Create: `experiment/kernels/SELECTION.md`
- Create: `experiment/kernels/src/vint.rs`, `src/index_framing.rs`, `src/bti_sparse_find.rs`, update `src/lib.rs`
- Create: `experiment/kernels/CARDS/vint.md`, `CARDS/index_framing.md`, `CARDS/bti_sparse_find.md`
- Test: `experiment/kernels/tests/shims.rs`, `experiment/kernels/tests/verbatim.rs`

**Interfaces:**
- Consumes: `experiment/results/profile/*.json` (Task 2), `work/cqlite/` (Task 2).
- Produces: the kernel C ABIs in README "Shared interfaces" (`msx_vint`, `msx_index_framing`, `msx_bti_sparse_find`, and each `msx_<kernel>_chain`), plus Rust-callable versions of the same functions in the `rlib` for Task 4's tests.

The kernels are **fixed** (spec section 4): `vint`, `index_framing`, `bti_sparse_find`. Do not re-select.

- [ ] **Step 1: Write `SELECTION.md`**

A record for the write-up, not a decision. It contains:

1. The three kernels, each with its source location and its share of samples in each bench from Task 2's `top_functions` (write "below 0.01%" or "not sampled" where that is the case).
2. The row/cell table: from Task 2's `read.json`, the top 15 `own_decoding` functions whose source is under `storage/sstable/reader/parsing/`, each with its share and the admissibility clause it fails, stated specifically ("returns `Result<Value>`; allocates the value and, on error, a formatted message; about 300 lines"). Read each function in `work/cqlite` to write its row.
3. The preliminary figures from spec section 4.2 beside Task 2's figures for the same functions.
4. One paragraph: the row/cell category is unfilled, and why.

If Task 2's profile shows a parsing function with at least 1% share that **does** meet every clause of the admissibility contract, add it under a heading "Admissible but not selected" with its numbers. The kernels still do not change; the write-up will mention it as a follow-up.

- [ ] **Step 2: Write the failing shim tests**

`tests/shims.rs`, for `vint` (prefill `out` with `0xDEAD_BEEF_DEAD_BEEF` before every call):

| Input bytes | Return | `*out` after |
|---|---|---|
| `[0x00]` | 1 | 0 |
| `[0x7F]` | 1 | 127 |
| `[0x80, 0x80]` | 2 | 128 |
| `[0xBF, 0xFF]` | 2 | 0x3FFF |
| `[0xFE, 1, 2, 3, 4, 5, 6, 7]` | 8 | 0x0001_0203_0405_0607 |
| `[0xFF, 1, 2, 3, 4, 5, 6, 7, 8]` | 9 | 0x0102_0304_0506_0708 |
| `[0x00, 0xAA]` (extra trailing byte) | 1 | 0 |
| `[]` | 0 | sentinel unchanged |
| `[0x80]` | 0 | sentinel unchanged |
| `[0xFF, 0, 0, 0, 0, 0, 0, 0]` | 0 | sentinel unchanged |

`msx_vint_chain` over `[0x00, 0x80, 0x80, 0x7F]` returns 3 operations. The chain checksum for every kernel is `acc = acc.rotate_left(5) ^ word`, folded over each operation's output words in order, starting from 0; assert the exact value computed by hand in the test.

For `index_framing` (`parse_vuint` is a thin adapter over `decode_unsigned`; every failure is `nom::Err::Error` with `ErrorKind::Eof`, so the error status is `1 + (ErrorKind::Eof as u32)`):

| Input | Return | `*out` |
|---|---|---|
| `[0x00, 0x02, b'a', b'b', 0x05, 0x00]` | 0 | `framing_len = 6`, `promoted_len = 0` |
| the same with a trailing `0x99` | 0 | `framing_len = 6`, `promoted_len = 0` |
| `[0x00, 0x00, 0x05, 0x81, 0x00]` (empty key, 2-byte promoted length) | 0 | `framing_len = 5`, `promoted_len = 256` |
| every proper prefix of the first row (lengths 0..=5) | `1 + (ErrorKind::Eof as u32)` | sentinel unchanged |

Plus a cross-check test: for 10,000 seeded random byte strings of length 0..=40, the shim's result equals a direct call to the copied CQLite function translated by the documented mapping.

For `bti_sparse_find`, with transitions `[(0x10, 100), (0x20, 200), (0x30, 300)]` (byte, child) and padding bytes filled with `0xA5`:

| `n` | `byte` argument | Return | `*out` |
|---|---|---|---|
| 3 | `0x10` | 1 | 100 |
| 3 | `0x20` | 1 | 200 |
| 3 | `0x30` | 1 | 300 |
| 3 | `0x0F`, `0x11`, `0x2F`, `0x31`, `0xFF` | 0 | sentinel unchanged |
| 3 | `0xABCD_EF20` (garbage above the low byte) | 1 | 200 |
| 0 | `0x10` | 0 | sentinel unchanged |
| 1 (first element only) | `0x10` | 1 | 100 |

Plus a cross-check against `slice::binary_search_by_key` for 10,000 seeded random sorted transition lists of length 0..=256.

If a row in these tables disagrees with the copied CQLite function, CQLite is right: correct the row and say so in the commit message. Never change copied code to match a table.

Run: `. experiment/env.sh && msx_cargo dev test --manifest-path experiment/Cargo.toml -p msx-kernels` — expected: compile failure (functions missing).

- [ ] **Step 3: Copy CQLite's code and write the shims**

Rules for copying:
- Copy each CQLite function, and the helpers it calls, **verbatim** between marker comments:
  ```rust
  // BEGIN VERBATIM cqlite-core/src/parser/vint.rs :: decode_unsigned @ 2e5b7a4c
  …
  // END VERBATIM
  ```
- The only permitted edits inside a verbatim block are removing or changing a visibility keyword and removing `debug!`/`trace!` logging macro calls. List any such removal in the kernel's card.
- `index_framing` needs `decode_unsigned`, `VIntError`, `parse_vuint`, and `parse_big_index_entry_framing`, with `nom = "=7.1.3"` (the version in CQLite's lockfile; `be_u16` and `take` come from it).
- `bti_sparse_find` copies the `Sparse` arm of `BtiNode::find_child`: the expression `transitions.binary_search_by_key(&byte, |t| t.byte).ok().map(|idx| &transitions[idx].child)`. CQLite's `Transition` is `{ byte: u8, child: SizedPointer }` with `SizedPointer { distance: u64 }` and no `repr`; the kernel uses the `#[repr(C)]` `Transition { byte: u8, child: u64 }` from the README, which has the same 16-byte shape. Put the verbatim markers around the search expression and state this type substitution in the card.
- Each shim is a `#[no_mangle] pub unsafe extern "C" fn` that builds the slice from its arguments, calls the verbatim code, and translates the result. It writes `*out` only on success. `msx_bti_sparse_find` uses `byte as u8`.
- Each `msx_<kernel>_chain(seg, seg_len, checksum)` loops the verbatim function over the segment exactly as the harness's `run_chain` will (Task 5 mirrors this loop): start at offset 0; call; on error stop; fold outputs into the checksum; advance. Advance rules: `vint` advances by the bytes consumed; `index_framing` advances by `framing_len + promoted_len` and stops if that passes the end; `bti_sparse_find` walks records (format in Task 4), folds the return value and, when found, the child, and advances to the next record. The verbatim function must be inlinable into this loop (`#[inline]` as in CQLite, or `#[inline]` added on the copy and noted in the card).

`tests/verbatim.rs`: for each marker block, strip visibility keywords and logging lines from both sides, normalize whitespace, and assert the block's text is a substring of the named file in `work/cqlite`. The test is skipped with a printed notice if `work/cqlite` is absent.

- [ ] **Step 4: Run the tests**

Run: `msx_cargo dev test --manifest-path experiment/Cargo.toml -p msx-kernels`
Expected: all pass.

- [ ] **Step 5: Record what the compiler produced**

```bash
msx_cargo admiss build --release --manifest-path experiment/Cargo.toml -p msx-kernels
"$MSX_LLVM_BIN/llvm-objdump" -d --no-show-raw-insn work/target/admiss/release/libmsx_kernels.so > work/admiss.dis
```

For each `msx_<kernel>` (not the `_chain` functions), list every `call` and every `jmp` that leaves the function:

- Expected for `msx_vint`: exactly one, an indirect call through the global offset table to `memcpy` (the variable-length `copy_from_slice`). This was observed on 12 November 2026 and is allowed by the admissibility contract.
- Allowed for any kernel: calls to `memcpy`, `memmove`, `memset`, `memcmp`, `bcmp`.
- Anything else (in particular a call into `core::panicking` or `core::slice::index`) is a possible panic. Work out whether any input can reach it:
  - no input can (the compiler just failed to prove it unreachable): note it in the card and continue;
  - some input can: add that input to the shim tests, record the finding in `SELECTION.md`, and exclude that class of input from the case generators in Task 5, stating the exclusion in the card's contract. The kernel stays.

Record in each card: the kernel's code size in bytes, and its outgoing calls.

- [ ] **Step 6: Write the kernel cards**

One Markdown file per kernel, written for an agent that has never seen CQLite. Sections, in this order:

1. **What it does**: two or three sentences, plus the on-disk format it parses.
2. **C signature**: exactly as exported.
3. **Contract**: what is returned and written for success and for each error; "nothing is written to `*out` on error".
4. **What you may assume**: only what is always true. For `vint` and `index_framing`: nothing beyond the signature. For `bti_sparse_find`: the transitions are sorted ascending by `byte` with no duplicates, and `n ≤ 256`.
5. **What you may NOT assume**: the input has no readable bytes before `ptr` or at or after `ptr + len` (for `bti_sparse_find`: before `t` or at or after `t + n`); the length may be 0; the pointer has no alignment beyond the type's; bytes may have any value; struct padding is unspecified; for `bti_sparse_find`, the upper 24 bits of `byte` are garbage.
6. **How CQLite calls it**: the chain or record pattern the measurement replays.
7. **Input statistics**: left as a placeholder heading; filled in by Task 4.
8. **Source**: the verbatim Rust, any logging lines removed, and any type substitution.
9. **This CPU**: Zen 5, with the feature list from `lscpu` that matters (BMI1, BMI2, LZCNT, POPCNT, MOVBE, AVX2, AVX-512 F/BW/VL/DQ/VBMI/VBMI2). Any instruction this CPU supports is allowed.

The cards describe the contract and the inputs. They do not comment on the quality of the compiler's code or suggest optimizations; in particular they do not mention the `memcpy` call. Agents get the compiler's assembly separately and draw their own conclusions.

- [ ] **Step 7: Commit**

```bash
git add experiment/kernels && git commit -m "feat: msx-kernels crate and kernel cards for vint, index_framing, bti_sparse_find"
```

---

### Task 4: Trace capture, search/holdout split, manifest

**Files:**
- Create: `experiment/trace-capture/cqlite-trace.patch`, `experiment/trace-capture/capture.sh`, `experiment/trace-capture/README.md`
- Create: `experiment/kernels/src/bin/msx-trace-manifest.rs`
- Create: `experiment/traces/<kernel>/{search,holdout}/*.seg`, `experiment/traces/<kernel>/MANIFEST.json`
- Modify: `experiment/kernels/CARDS/*.md` (section 7, input statistics)
- Test: `experiment/kernels/tests/traces.rs`

**Interfaces:**
- Consumes: the Rust-callable kernel functions from Task 3.
- Produces: segment files and manifests, consumed by Tasks 5-13.

**Segment formats** (a segment is one file, `*.seg`):

| Kernel | Format |
|---|---|
| `vint` | stream: the raw varint bytes, concatenated, walked as a chain |
| `index_framing` | stream: one whole BIG-format `Index.db` file |
| `bti_sparse_find` | records: repeated `[u32 LE n][u32 LE byte][n × 16 bytes]`, each 16-byte element being `[byte][7 zero bytes][u64 LE child]` |

**`MANIFEST.json`:**

```json
{
  "kernel": "vint",
  "cqlite_commit": "2e5b7a4c…",
  "capture_commands": ["…"],
  "format": "stream",
  "segments": [ { "file": "search/0007.seg", "origin": "read/full_scan", "split": "search",
                  "sha256": "…", "bytes": 0, "ops": 0 } ],
  "excluded": [ { "origin": "…", "reason": "walk ended 3 bytes before end of file" } ],
  "duplicates_dropped": 0,
  "totals": { "search_ops": 0, "holdout_ops": 0, "search_bytes": 0, "holdout_bytes": 0 },
  "small_trace": { "search": false, "holdout": false },
  "histograms": { }
}
```

`origin` is the Criterion case that produced the segment (`<group>/<case>`), or for `index_framing` the `Index.db` path relative to the datasets root. `histograms`: for `vint`, the count of operations at each encoded length 1..=9; for `index_framing`, key-length and promoted-length quantiles; for `bti_sparse_find`, the distribution of `n` and the hit rate.

**Split rule:** a segment goes to `holdout` if the first byte of the SHA-256 of its **content** is below 77 (about 30%), else `search`. Segments with identical content are deduplicated first (keep one, count the rest in `duplicates_dropped`).

**Where the inputs come from** (decided from what exists locally):

- `index_framing`: every `*-big-Index.db` under `/home/dev/projects/cqlite/test-data/datasets`. No instrumentation. On 12 November 2026 there were 152 such files; 151 replayed as a clean chain, for 4,629 entries in total.
- `vint` and `bti_sparse_find`: recorded from CQLite's own benchmark cases, run once each in Criterion's smoke mode, from an instrumented build of the clone. The cases are every case of the benches `read`, `partition_lookup` and `m1_performance` (list them with `<binary> --bench --list`). One process per case, so each case's inputs land in their own segments.

- [ ] **Step 1: Write the failing trace tests**

`tests/traces.rs`:

- `manifest_matches_segments`: for every kernel directory under `experiment/traces/`, every segment listed exists, its SHA-256 and byte count match, and walking it with the Rust-level kernel function yields exactly the listed `ops` and consumes the whole segment.
- `no_duplicate_content`: no two segments of a kernel have the same SHA-256.
- `stream_with_trailing_garbage_is_rejected`: build in memory a valid `index_framing` stream of three entries followed by `[0xFF]`; assert the manifest tool's `walk` function reports failure with a reason containing "before end" or "error", and does not report 3 ops as a success.
- `split_rule_is_deterministic`: `split_for(content)` returns the same value twice and matches the rule above for two hand-computed examples.
- `index_framing_counts`: the `index_framing` manifest has at least 150 segments and `search_ops + holdout_ops` between 4,000 and 5,500. (Sanity bounds around the observed 4,629; if the datasets changed, update the bounds and say so.)

Run: `msx_cargo dev test --manifest-path experiment/Cargo.toml -p msx-kernels --test traces` — expected: fails.

- [ ] **Step 2: Write the capture patch (applied to the clone only)**

The patch adds a module `msx_trace` to `work/cqlite/cqlite-core` and two call sites. Behavior:

- Inactive unless the environment variable `MSX_TRACE_DIR` is set. When inactive the hook is one branch on a `OnceLock<bool>`.
- Per thread, an in-memory buffer per kernel; flushed to `$MSX_TRACE_DIR/<kernel>.<thread-seq>.bin` when the thread exits and at process exit.
- `vint`: in `decode_unsigned` (`cqlite-core/src/parser/vint.rs`), on every **successful** decode, append exactly the bytes consumed (`&input[..total]`). Concatenated, these form a valid stream with the real value order.
- `bti_sparse_find`: in `BtiNode::find_child` (`cqlite-core/src/storage/sstable/bti/node.rs`), in the `Sparse` arm, append one record in the format above, serializing each transition explicitly (do not transmute CQLite's struct).

Save the diff as `experiment/trace-capture/cqlite-trace.patch` (`git -C work/cqlite diff > …`). The patch touches only the clone.

- [ ] **Step 3: Write `capture.sh` and capture**

`capture.sh`:
1. applies the patch to the clone if not already applied;
2. builds the three benches from the patched clone into `work/target/cqlite-trace`:
   `cargo bench -p cqlite-core --profile perfsym --features cli-helpers,bench-internals --bench read --bench partition_lookup --bench m1_performance --no-run`;
3. for each bench, lists its cases with `<binary> --bench --list`, and for each case runs
   `MSX_TRACE_DIR=work/traces-raw/<bench>/<case-slug>/ CQLITE_DATASETS_ROOT=/home/dev/projects/cqlite/test-data/datasets <binary> --bench --test '^<case id>$'`.
   `--test` runs each selected case once. A case that fails is skipped and listed with its error;
4. records every command line in `experiment/trace-capture/README.md`.

- [ ] **Step 4: Write `msx-trace-manifest` and build the segment trees**

`msx-trace-manifest --raw work/traces-raw --datasets /home/dev/projects/cqlite/test-data/datasets --out experiment/traces`:
- for `vint` and `bti_sparse_find`, each non-empty per-thread raw file becomes one candidate segment, with `origin` set to its case;
- for `index_framing`, each `*-big-Index.db` becomes one candidate segment **only if** the chain walk ends exactly at the end of the file with no error; otherwise it goes to `excluded` with the reason;
- deduplicates by content, applies the split rule, names segments `NNNN.seg` in sorted (`origin`, SHA-256) order, and writes `MANIFEST.json` with histograms, totals and `small_trace`.

`small_trace.<split>` is true when the split, cut into chunks of `SAMPLE_MIN_OPS` (50,000) operations, yields fewer than `SMALL_TRACE_MIN_CHUNKS` (20) chunks, that is, when the split has under 1,000,000 operations.

Expose the walk as a library function `walk(kernel, bytes) -> Result<u64 /*ops*/, String>` used by both the tool and the tests.

- [ ] **Step 5: Run the trace tests**

Run: `msx_cargo dev test --manifest-path experiment/Cargo.toml -p msx-kernels --test traces`
Expected: all pass.

- [ ] **Step 6: Apply the size rules and fill in the cards**

These rules are fixed; apply them, do not improvise:

| Situation, per kernel | What to do |
|---|---|
| a split has `small_trace` true | nothing here: Task 8 measures it in small-trace mode. Add "Inputs repeat within a sample (small trace: N operations)" to the card |
| `holdout_ops` is zero or under 10% of the kernel's operations | set that kernel's split threshold byte to the smallest value that puts at least 20% of operations in holdout; record it as `"split_threshold"` in the manifest |
| `index_framing` | expected: both splits small (about 3,200 search and 1,400 holdout operations) |
| `bti_sparse_find` has fewer than 2,000 operations in total | the kernel is **dropped**: delete its traces directory, mark it "dropped: only N recorded calls in the local data" in `SELECTION.md`, and run the rest of the plan with two kernels. Keep its crate code and card. Do not synthesize inputs |
| `vint` has fewer than 200,000 operations in total | continue; add "Small trace: N operations" to the card. It becomes a stated limitation |

Do not pad any trace with synthetic or repeated data.

Fill in section 7 of each card from the **search** split only: the histograms, in a small table, and one sentence on what dominates (for example "93% of varints are one byte").

- [ ] **Step 7: Commit**

```bash
git add experiment/trace-capture experiment/traces experiment/kernels && git commit -m "feat: recorded input traces with search/holdout split"
```
