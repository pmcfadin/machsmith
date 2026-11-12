# Kernel selection record

This is a record for the write-up, not a decision. The three kernels were fixed on
12 November 2026 (spec section 4) and are not re-selected here. Figures are shares of
user-space cycle samples from Task 2 (`experiment/results/profile/*.json`, CQLite
commit `2e5b7a4cf6abe6f51b7677ecb7199e2cab087c0e`, `perf record -e cycles:u -F 4999`).

## 1. The three kernels

| kernel | CQLite source | read | partition_lookup | decode |
|---|---|---|---|---|
| `vint` | `cqlite-core/src/parser/vint.rs` :: `decode_unsigned` | 0.09% | not sampled | 0.04% |
| `index_framing` | `cqlite-core/src/storage/sstable/index_reader/parse.rs` :: `parse_big_index_entry_framing` (calls `parse_vuint`, which calls `decode_unsigned`) | below 0.09% (not in `own_decoding_top`; the 15th entry is 0.09%) | not sampled (`own_decoding_top` holds only `IndexReader::lookup_partition`) | below 0.01% (not in `own_decoding_top`; the 15th entry is 0.0002%) |
| `bti_sparse_find` | `cqlite-core/src/storage/sstable/bti/node.rs` :: `BtiNode::find_child`, `Sparse` arm | below 0.09% (not listed) | not sampled | below 0.01% (not listed) |

None of the three is among the top 40 functions of any bench. The profile does not
make a case for them as hot spots; they were chosen for being small, admissible and
representative of three decoder shapes (a varint, a nom-framed index entry, a binary
search). The honest reading is the ceiling in `results/profile/ceiling.md`:
`own_decoding` is at most 5.5% to 5.8% of user-space time in every bench.

## 2. Row/cell functions and the admissibility clause each fails

The top 15 `own_decoding` functions of the `read` bench that live under
`storage/sstable/reader/parsing/`. `read.json` lists 15 `own_decoding` entries, one of
which (`decode_unsigned`, `parser/vint.rs`) is not under `parsing/`, so 14 rows
qualify. Line counts are function spans measured in `work/cqlite`.

| share (read) | function | source (under `reader/parsing/`) | clause failed |
|---|---|---|---|
| 0.81% | `parse_value_from_raw_bytes_reporting` | `row_decoder/raw_value/reporting.rs` | returns `Result<(Value, usize)>`; takes `&str` type and column names; allocates the `Value` and formatted messages; about 470 lines |
| 0.70% | `parse_clustering_prefix` | `row_decoder/row_framing.rs` | takes `&TableSchema`; returns `Result<(Vec<Value>, usize)>`; allocates a `Vec` and the values; about 115 lines |
| 0.64% | `checked_vuint_exact_length` | `row_decoder/vuint_length.rs` | 9 lines delegating to `checked_vuint_exact_length_as`, which on error builds a `Vec<String>`, joins it and formats an `Error::corruption` message; takes three `&str`; returns `Result<usize>`. The success path is a comparison against one or two sizes |
| 0.52% | `parse_row_data_with_offset_impl` | `row_decoder/row_data.rs` | takes `&TableSchema` and `&SSTableReader`; builds rows; allocates; about 870 lines |
| 0.46% | `decode_scalar_cell_value` | `row_decoder/cell_value_scalar.rs` | takes `&Column` and `&CellKind`; returns `Result<Value>`; allocates the value and error messages; about 565 lines |
| 0.31% | `parse_block_emit_windowed` (`bti_collect_partition_rows` closure instance) | `row_decoder/block_emit_windowed.rs` | generic over a closure callback, takes the reader and schema; emits owned rows; about 720 lines |
| 0.28% | `is_complex_column` | `row_decoder/udt.rs` | takes `&str`; `to_lowercase()` allocates a `String`; string-prefix matching; about 45 lines. Not C-ABI shaped without rewriting |
| 0.27% | `CellKind::from_type` | `row_decoder/cell_kind.rs` | takes `&str`; `to_lowercase()` allocates; matches type-name strings; about 26 lines. Not C-ABI shaped without rewriting |
| 0.22% | `parse_cell_path_key_reporting` | `row_decoder/complex_column/cell_path_key.rs` | returns `Result<Value>`; takes `&str` type and column names and a `&mut bool`; allocates; about 130 lines |
| 0.17% | `parse_complex_column_inner` | `row_decoder/complex_column.rs` | takes `&Column`, `&str`; returns owned collections; allocates; about 770 lines |
| 0.14% | `decode_reporting_consumption` | `row_decoder/complex_column/cell_path_key.rs` | returns `Result<(Value, Option<usize>)>`; allocates; about 115 lines |
| 0.10% | `parse_complex_cell_value` | `row_decoder/complex_column.rs` | takes `&Column`, `&str`; returns `Value`s; allocates; about 235 lines |
| 0.09% | `parse_block_emit_windowed` (`parse_block` closure instance) | `row_decoder/block_emit_windowed.rs` | as the other instance; about 720 lines |
| 0.09% | `block_partition_header` | `row_decoder/block_emit_windowed/partition_header_arm.rs` | returns `Result<HeaderStep>`; error path allocates; about 145 lines |

For the `decode` bench the only `own_decoding` function of note is
`parse_value_with_comparator_at_depth` (4.88%, `comparator_value_parsing.rs`, about 270
lines): it takes a `&ComparatorType`, returns `Result<Value>` and allocates. The next,
`decode_custom_scalar` (0.53%, about 47 lines), takes a `&str` name and returns
`Result<Value>`; the `Value` allocates for string and collection arms. Neither is
admissible. The `partition_lookup` bench has no row/cell function; its single
`own_decoding` entry (5.56%) is `IndexReader::lookup_partition`, a hash-map lookup.

## 3. Preliminary figures (spec section 4.2) beside Task 2's

| function | spec 4.2 (preliminary) | Task 2, `read` |
|---|---|---|
| `parse_row_data_with_offset_impl` | 7.2% | 0.52% |
| `parse_clustering_prefix` | 2.8% | 0.70% |
| `parse_row_metadata` | 1.9% | not in top 15 (below 0.09%) |
| `parse_cell_value_schema_order` | 1.5% | not in top 15 (below 0.09%) |
| `parse_value_from_raw_bytes_reporting` | 1.2% | 0.81% |
| `parse_complex_column_inner` | 1.0% | 0.17% |
| `checked_vuint_exact_length` | 0.6% | 0.64% |

The full method attributes samples to the innermost function after resolving inlining,
so inlined helpers and callees (`Arc::clone`, hashing, slice code) land in other
buckets rather than in the parser that called them. That lowers the large functions
(`parse_row_data_with_offset_impl` most of all) and leaves the small standalone one
(`checked_vuint_exact_length`) unchanged. The preliminary run attributed to the
enclosing function. Both sets agree on the order of magnitude: each row/cell function is
a few percent of samples at most.

## 4. The row/cell category is unfilled

No row/cell function meets the admissibility contract. Every function that carries
time in the row path is hundreds of lines, takes CQLite's schema, column or reader
types, returns `Value` or `Result` and allocates both its values and its error messages.
The one small function with a real share, `checked_vuint_exact_length`, is a comparison
with a one- or two-element list wrapped in an error path that allocates a message;
the two small helpers (`is_complex_column`, `CellKind::from_type`) allocate a lowercased
`String` per call. Putting any of them behind a C boundary would mean rewriting them,
and a rewrite is no longer CQLite's code. The category stays unfilled and the write-up
says so.

## Admissible but not selected

None. No parsing function with a share of at least 1% meets every clause of the
contract: the only one at that level, `parse_value_with_comparator_at_depth` (4.88% in
`decode`), takes a `&ComparatorType`, returns `Result<Value>` and allocates.

## 5. Panic reachability

See the kernel cards. In the build of 12 November 2026, `msx_index_framing` contains
two panic call sites (`split_at` in nom's `take`, and a slice-range index for the
remaining input after the second varint); both are guarded by checks the compiler
did not fold away, and neither is reachable from any input. No input class is excluded
from the case generators.

## 6. Trace capture outcome (Task 4)

- `bti_sparse_find`: **dropped: only 0 recorded calls in the local data.** The hook in
  `BtiNode::find_child` (`Sparse` arm) never fired in any of the 23 traced bench cases:
  in CQLite at `2e5b7a4c` that method is called only from unit tests; the read path
  descends the trie with `find_child_offset`. Its crate code and card are kept; no
  traces directory exists; the rest of the plan runs with two kernels (`vint`,
  `index_framing`). No inputs were synthesized.
- `vint`: 1,182,791 recorded operations (search 1,024,899, holdout 157,892); above the
  200,000 limit, no limitation; the holdout split is a small trace.
- `index_framing`: 4,551 entries after deduplication (search 1,920, holdout 2,631), both
  splits small traces.

