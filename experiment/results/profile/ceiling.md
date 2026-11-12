# CQLite ceiling report

Host: AMD Ryzen AI MAX+ 395 (Zen 5), Linux 7.0.0-42-generic, 12 November 2026. CQLite commit `2e5b7a4cf6abe6f51b7677ecb7199e2cab087c0e`, built with its `perfprof` profile (release code generation, line tables, unstripped), rustc 1.97.1.

Shares are percentages of user-space cycle samples (`perf record -e cycles:u -F 4999`, one pinned core). Rows sum to 100.

| bench | samples | own_decoding | allocator | memcpy | decompression | checksum | sync | other_cqlite | bench_harness | other |
|---|---|---|---|---|---|---|---|---|---|---|
| read | 191,607 | 5.8 | 30.6 | 7.3 | 0.4 | 0.0 | 1.7 | 7.5 | 2.4 | 44.3 |
| partition_lookup | 119,876 | 5.6 | 0.1 | 0.0 | 0.0 | 0.0 | 0.0 | 0.0 | 0.2 | 94.2 |
| decode | 620,706 | 5.5 | 3.4 | 0.1 | 0.0 | 0.0 | 0.0 | 13.6 | 0.3 | 77.1 |

## Reading the ceiling

`own_decoding` is the ceiling: the most that perfect decoding kernels could speed up each workload.

- **read** (9 cases). `read` exercises SSTable reads end to end (9 cases: `get_partition_big`/`_bti`, `point_lookup_repeated`, `clustering_slice`/`_point_bti`/`_range_bti`, `full_scan`, `type_heavy`, `scan_partition_dense_stream`); allocation and memory copy dominate. If every decoding kernel were infinitely fast, `read` would speed up by at most 5.8% of its user-space time (a factor of 1.06x).
- **partition_lookup** (6 cases). `partition_lookup` measures a hash-map lookup of an already-loaded index (SipHash plus a byte comparison), not index parsing. If every decoding kernel were infinitely fast, `partition_lookup` would speed up by at most 5.6% of its user-space time (a factor of 1.06x).
- **decode** (31 cases). `decode` spends most of its time parsing CQL type-name strings (`CqlType::parse_with_depth` and the comparator it feeds), not SSTable bytes. If every decoding kernel were infinitely fast, `decode` would speed up by at most 5.5% of its user-space time (a factor of 1.06x).

Caveats that affect how to read the table:

- Samples are attributed to the innermost function after resolving inlining (`addr2line -i`). Inlined standard-library and dependency code (for example `Arc::clone`, `core::hash::sip`, slice equality) lands in `other` even when a decoder calls it, so `own_decoding` is a lower bound on time spent inside decoding call trees. Read the `other` column with that in mind.
- `partition_lookup`'s `own_decoding` figure is `IndexReader::lookup_partition`, which the bucket rule counts because it lives under `index_reader`; the function is a hash-map lookup, not a parser.
- libc is stripped here. Its functions are named from the dynamic symbols, IFUNC targets as resolved on this CPU, and `.eh_frame` function boundaries. Unnamed functions inside the span of the malloc family's exported symbols are counted as `allocator` (glibc's `_int_malloc`, `_int_free`, tcache). `memcpy` holds memmove, memcpy, mempcpy and memset only; `bcmp`/`memcmp` go to `other` (`__bcmp` is about 20% of `partition_lookup` and is a comparison, not a copy).
- Kernel-address samples that leaked into the user-only profile (about 0.4%) are dropped. Criterion's in-process profiler (`bench_harness`) is not counted as CQLite's cost.

## Top 15 `own_decoding` functions

### read

| share | function | source |
|---|---|---|
| 0.81% | `<cqlite_core::storage::sstable::reader::parsing::row_decoder::V5CompressedLegacyParser>::parse_value_from_raw_bytes_reporting` | `src/storage/sstable/reader/parsing/row_decoder/raw_value/reporting.rs` |
| 0.70% | `<cqlite_core::storage::sstable::reader::parsing::row_decoder::V5CompressedLegacyParser>::parse_clustering_prefix` | `src/storage/sstable/reader/parsing/row_decoder/row_framing.rs` |
| 0.64% | `cqlite_core::storage::sstable::reader::parsing::row_decoder::vuint_length::checked_vuint_exact_length` | `src/storage/sstable/reader/parsing/row_decoder/vuint_length.rs` |
| 0.52% | `<cqlite_core::storage::sstable::reader::parsing::row_decoder::V5CompressedLegacyParser>::parse_row_data_with_offset_impl` | `src/storage/sstable/reader/parsing/row_decoder/row_data.rs` |
| 0.46% | `<cqlite_core::storage::sstable::reader::parsing::row_decoder::V5CompressedLegacyParser>::decode_scalar_cell_value` | `src/storage/sstable/reader/parsing/row_decoder/cell_value_scalar.rs` |
| 0.31% | `<cqlite_core::storage::sstable::reader::parsing::row_decoder::V5CompressedLegacyParser>::parse_block_emit_windowed::<<cqlite_core::storage::sstable::reader::types::SSTableReader>::bti_collect_partition_rows::{closure#2}>` | `src/storage/sstable/reader/parsing/row_decoder/block_emit_windowed.rs` |
| 0.28% | `<cqlite_core::storage::sstable::reader::parsing::row_decoder::V5CompressedLegacyParser>::is_complex_column` | `src/storage/sstable/reader/parsing/row_decoder/udt.rs` |
| 0.27% | `<cqlite_core::storage::sstable::reader::parsing::row_decoder::cell_kind::CellKind>::from_type` | `src/storage/sstable/reader/parsing/row_decoder/cell_kind.rs` |
| 0.22% | `<cqlite_core::storage::sstable::reader::parsing::row_decoder::V5CompressedLegacyParser>::parse_cell_path_key_reporting` | `src/storage/sstable/reader/parsing/row_decoder/complex_column/cell_path_key.rs` |
| 0.17% | `<cqlite_core::storage::sstable::reader::parsing::row_decoder::V5CompressedLegacyParser>::parse_complex_column_inner` | `src/storage/sstable/reader/parsing/row_decoder/complex_column.rs` |
| 0.14% | `<cqlite_core::storage::sstable::reader::parsing::row_decoder::V5CompressedLegacyParser>::decode_reporting_consumption` | `src/storage/sstable/reader/parsing/row_decoder/complex_column/cell_path_key.rs` |
| 0.10% | `<cqlite_core::storage::sstable::reader::parsing::row_decoder::V5CompressedLegacyParser>::parse_complex_cell_value` | `src/storage/sstable/reader/parsing/row_decoder/complex_column.rs` |
| 0.09% | `<cqlite_core::storage::sstable::reader::parsing::row_decoder::V5CompressedLegacyParser>::parse_block_emit_windowed::<<cqlite_core::storage::sstable::reader::parsing::row_decoder::V5CompressedLegacyParser>::parse_block::{closure#0}>` | `src/storage/sstable/reader/parsing/row_decoder/block_emit_windowed.rs` |
| 0.09% | `cqlite_core::parser::vint::decode_unsigned` | `src/parser/vint.rs` |
| 0.09% | `<cqlite_core::storage::sstable::reader::parsing::row_decoder::V5CompressedLegacyParser>::block_partition_header` | `src/storage/sstable/reader/parsing/row_decoder/block_emit_windowed/partition_header_arm.rs` |

### partition_lookup

| share | function | source |
|---|---|---|
| 5.56% | `<cqlite_core::storage::sstable::index_reader::IndexReader>::lookup_partition` | `/rustc/8bab26f4f68e0e26f0bb7960be334d5b520ea452/library/core/src/slice/index.rs` |

### decode

| share | function | source |
|---|---|---|
| 4.88% | `cqlite_core::storage::sstable::reader::parsing::comparator_value_parsing::parse_value_with_comparator_at_depth` | `src/storage/sstable/reader/parsing/comparator_value_parsing.rs` |
| 0.53% | `cqlite_core::storage::sstable::reader::parsing::custom_scalar::decode_custom_scalar` | `src/storage/sstable/reader/parsing/custom_scalar.rs` |
| 0.04% | `cqlite_core::parser::vint::decode_unsigned` | `src/parser/vint.rs` |
| 0.01% | `cqlite_core::parser::vint::zigzag_decode` | `src/parser/vint.rs` |
| 0.00% | `cqlite_core::storage::sstable::reader::parsing::comparator_value_parsing::parse_value_with_comparator` | `src/storage/sstable/reader/parsing/comparator_value_parsing.rs` |
| 0.00% | `cqlite_core::storage::sstable::reader::parsing::value_parsing::parse_tuple_value_with::<<cqlite_core::storage::sstable::reader::types::SSTableReader>::parse_tuple_value::{closure#0}>` | `/rustc/8bab26f4f68e0e26f0bb7960be334d5b520ea452/library/core/src/slice/iter/macros.rs` |
| 0.00% | `cqlite_core::storage::sstable::reader::parsing::frozen_value_parsing::read_element` | `src/storage/sstable/reader/parsing/frozen_value_parsing.rs` |
| 0.00% | `cqlite_core::storage::sstable::reader::parsing::value_parsing::parse_list_value_with::<<cqlite_core::storage::sstable::reader::types::SSTableReader>::parse_list_value::{closure#0}>` | `src/storage/sstable/reader/parsing/value_parsing.rs` |
| 0.00% | `cqlite_core::storage::sstable::reader::parsing::frozen_value_parsing::decode_elements::<cqlite_core::storage::sstable::reader::parsing::frozen_value_parsing::parse_frozen_inner_with::{closure#0}>` | `src/storage/sstable/reader/parsing/frozen_value_parsing.rs` |
| 0.00% | `cqlite_core::storage::sstable::reader::parsing::frozen_value_parsing::read_i32_be` | `src/storage/sstable/reader/parsing/frozen_value_parsing.rs` |
| 0.00% | `cqlite_core::storage::sstable::reader::parsing::frozen_value_parsing::read_count` | `src/storage/sstable/reader/parsing/frozen_value_parsing.rs` |
| 0.00% | `cqlite_core::parser::vint::parse_vint_length` | `src/parser/vint.rs` |
| 0.00% | `cqlite_core::storage::sstable::reader::parsing::comparator_value_parsing::parse_duration_value` | `src/lib.rs` |
| 0.00% | `cqlite_core::storage::sstable::reader::parsing::frozen_value_parsing::reject_trailing` | `src/storage/sstable/reader/parsing/frozen_value_parsing.rs` |
| 0.00% | `cqlite_core::storage::sstable::reader::parsing::frozen_value_parsing::parse_frozen_list_value_with::<cqlite_core::storage::sstable::reader::parsing::frozen_value_parsing::parse_frozen_inner_with::{closure#0}>` | `src/storage/sstable/reader/parsing/frozen_value_parsing.rs` |

## Commands

```bash
. experiment/env.sh
git clone --no-hardlinks /home/dev/projects/cqlite "$MSX_WORK/cqlite"
git -C "$MSX_WORK/cqlite" checkout --detach 2e5b7a4cf6abe6f51b7677ecb7199e2cab087c0e
for b in read partition_lookup decode; do
  bash tools/profile-cqlite.sh "$b"
  python3 tools/ceiling.py "$b" "work/profile/$b.script" "$(cat work/profile/$b.binpath)" "experiment/results/profile/$b.json"
done
```

`profile-cqlite.sh` builds with `cargo bench -p cqlite-core --profile perfprof --features cli-helpers,bench-internals --bench <bench> --no-run` (`CARGO_TARGET_DIR=work/target/cqlite-perfprof`) and runs `taskset -c 5 perf record -e cycles:u -F 4999 -- <binary> --bench --profile-time 4` with `CQLITE_DATASETS_ROOT=/home/dev/projects/cqlite/test-data/datasets`.
