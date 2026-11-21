# Round 2 hot spots (Task 9)

CQLite `7f9abe36`, `bench`-profile binaries from `binaries.json`. Ledger: profile A `R2-000011`,
profile B `R2-000012`, call-graph run A-cg `R2-000013`.

## How to read this

- **Self share** is the share of `cycles:u` samples whose innermost frame (after inline resolution)
  belongs to the hot spot. A and B are two flat profiles taken at different times (spec section 5).
- **Confirmed** (spec section 5): ≥ 3% in both A and B, and A and B differ by < 2 percentage points.
- A hot spot is a **group** of frames that one fix would act on. Inlining spreads one source-level
  cost over many frames (SipHash shows up as `rotate_left`, `wrapping_add` and `Sip13Rounds::*`), so
  single frames are the wrong unit. Each group's membership is listed below. Grouping is the
  controller's judgment. The single-frame view is in `{A,B}/<workload>.json` (`top_self`).
- **Inclusive (A-cg)** is from the `--call-graph dwarf` run and is structural evidence only. DWARF
  chains are mostly truncated: chains reach `main`/Criterion in 0.4% (read), 20.9% (decode) and 12.1%
  (partition_lookup) of samples, and the median chain depth is 1 to 2 frames. A-cg also samples at
  999 Hz, not 4999, on other cores, and its *self* shares do not match A's: read's allocator bucket is
  74.1% in A-cg against 31.2% in A. **No number in this file depends on A-cg** except the column
  labelled inclusive.
- **Target cases** for a hot spot: the cases where its share is ≥ 3% in both A and B.

## Profiles

`perf record -e cycles:u -F 4999`, pinned core, `--profile-time 20` per case.

| workload | cases | profile time | A start (core) | A samples | B start (core) | B samples | B − A |
|---|---|---|---|---|---|---|---|
| read | 9 | 9 × 20 s | 2026-11-13 08:18Z (5) | 915,373 | 2026-11-13 13:52Z (5) | 914,912 | 5.6 h |
| decode | 31 | 31 × 20 s | 2026-11-13 08:32Z (5) | 3,115,056 | 2026-11-13 21:02Z (5) | 3,196,030 | 12.5 h |
| partition_lookup | 6 | 6 × 20 s | 2026-11-13 09:07Z (13) | 554,577 | 2026-11-13 21:24Z (13) | 596,812 | 12.3 h |

Host at start, 1/5/15-minute load: A read 1.55/1.68/7.40, decode 0.87/1.20/3.73, partition_lookup
1.87/1.50/1.95. B read 1.03/0.94/1.19, decode 1.99/1.71/7.54, partition_lookup 0.72/1.02/2.79. No
`cargo`/`rustc` at any start. B for decode and partition_lookup is the rerun on A's cores (DECISIONS
R17); the first B is kept in `B-mixed-core/` and not used here.

A-cg (999 Hz, DWARF): read 50,787 samples (core 11), decode 426,650 (core 13), partition_lookup
115,974 (core 5).

Per-case samples:

**read**

| case | A samples | B samples |
|---|---|---|
| `read/clustering_point_bti` | 101,287 | 101,262 |
| `read/clustering_range_bti` | 102,481 | 99,198 |
| `read/clustering_slice` | 102,319 | 102,683 |
| `read/full_scan` | 100,372 | 99,797 |
| `read/get_partition_big` | 102,186 | 103,914 |
| `read/get_partition_bti` | 103,376 | 102,926 |
| `read/point_lookup_repeated` | 101,266 | 101,868 |
| `read/scan_partition_dense_stream` | 100,225 | 101,255 |
| `read/type_heavy` | 101,861 | 102,009 |
| total | 915,373 | 914,912 |

**decode**

| case | A samples | B samples |
|---|---|---|
| `decode/text_heavy` | 100,360 | 100,109 |
| `decode/type_ascii` | 101,081 | 136,362 |
| `decode/type_bigint` | 100,833 | 99,128 |
| `decode/type_blob` | 100,140 | 125,950 |
| `decode/type_boolean` | 99,577 | 99,950 |
| `decode/type_counter` | 99,474 | 102,262 |
| `decode/type_date` | 98,720 | 145,786 |
| `decode/type_decimal` | 99,749 | 99,892 |
| `decode/type_double` | 99,574 | 89,939 |
| `decode/type_duration` | 99,096 | 100,802 |
| `decode/type_float` | 98,647 | 99,865 |
| `decode/type_frozen` | 99,165 | 100,219 |
| `decode/type_inet` | 100,257 | 100,632 |
| `decode/type_int` | 100,110 | 98,369 |
| `decode/type_json` | 100,587 | 98,882 |
| `decode/type_list` | 99,079 | 98,256 |
| `decode/type_map` | 100,867 | 100,835 |
| `decode/type_set` | 100,644 | 91,981 |
| `decode/type_smallint` | 100,340 | 100,255 |
| `decode/type_text` | 99,934 | 83,116 |
| `decode/type_time` | 100,133 | 84,874 |
| `decode/type_timestamp` | 99,509 | 116,405 |
| `decode/type_timeuuid` | 99,075 | 91,692 |
| `decode/type_tinyint` | 100,199 | 98,747 |
| `decode/type_tuple` | 118,811 | 99,895 |
| `decode/type_udt` | 101,583 | 99,156 |
| `decode/type_uuid` | 99,093 | 104,133 |
| `decode/type_varchar` | 98,555 | 129,530 |
| `decode/type_varint` | 100,050 | 100,315 |
| `decode/vint_decode` | 98,790 | 99,727 |
| `decode/wide_row_primitives` | 101,024 | 98,966 |
| total | 3,115,056 | 3,196,030 |

**partition_lookup**

| case | A samples | B samples |
|---|---|---|
| `cache_operations/borrow_trait_lookup` | 61,632 | 100,307 |
| `index_lookup_cold/position_0` | 97,215 | 97,857 |
| `index_lookup_warm/hot_keys` | 99,742 | 98,893 |
| `lookup_distribution/random_access` | 104,435 | 98,923 |
| `lookup_distribution/sequential_access` | 90,005 | 101,253 |
| `lookup_throughput/1000_lookups` | 101,548 | 99,579 |
| total | 554,577 | 596,812 |

## Ranked hot spots

| rank | hot spot | workload | self A | self B | Δ pp | confirmed | inclusive (A-cg) | targets | fix route |
|---|---|---|---|---|---|---|---|---|---|
| 1 | H1 `key_lookup` HashMap get (SipHash-1-3 + probe + key compare) | partition_lookup | 95.96 | 97.22 | 1.27 | **yes** | `lookup_partition` 73.9 (sip 25.6, bcmp 33.0) | 6/6 | internal route exists (see H1) |
| 2 | H2 type string re-parsed per decoded value | decode | 64.31 | 61.12 | 3.19 | **no** (Δ ≥ 2 pp) | `decode_value_for_bench` 22.3, `parse_with_depth` 18.5, `from_type_string` 13.4 | 30/31 | internal |
| 3 | H3 allocator (malloc/free family) | read | 31.20 | 31.26 | 0.06 | **yes** | (74.6; unreliable, see above) | 9/9 | unknown: no call site |
| 4 | H4 per-cell `Arc<str>` column-name refcount | read | 11.17 | 11.22 | 0.04 | **yes** | `Arc<str>` clone/drop 0.9 (atomic not matched) | 8/9 | public type at the end of the path (see H4) |
| 5 | H5 memcpy | read | 5.04 | 5.14 | 0.10 | **yes** | — | 5/9 | unknown: no call site |
| 6 | H6 `parse_clustering_prefix` | read | 3.35 | 3.11 | 0.25 | **yes** | — | 2/9 | internal (`pub(crate)`) |

Shares are percent of the workload's samples. Everything below 3% in A or B is left out (e.g.
`Value::into_owned`, 2.61 / 2.52).

### H1: `IndexData::key_lookup` lookup (partition_lookup)

- **Frames:** physical functions `IndexReader::lookup_partition` (68.98 / 69.89, with
  `HashMap::get`, hashbrown probing and most SipHash rounds inlined), `Hasher<Sip13Rounds>::write`
  (17.97 / 14.40), `__bcmp_ifunc_target` (9.01 / 12.93).
- The split inside the group is not stable (SipHash write −3.6 pp, bcmp +3.9 pp from A to B), but
  the group is: 95.96 / 97.22.
- **Source:** `index_reader/mod.rs:283` `lookup_partition` → `key_lookup.get(key_digest)`, a
  `HashMap<Arc<[u8]>, usize>` with the default `RandomState` (SipHash-1-3).
- **Targets (A / B):** `index_lookup_cold/position_0` 98.61 / 98.17, `index_lookup_warm/hot_keys`
  96.63 / 97.46, `cache_operations/borrow_trait_lookup` 96.46 / 98.20, `lookup_throughput/1000_lookups`
  95.39 / 97.36, `lookup_distribution/sequential_access` 94.43 / 94.99,
  `lookup_distribution/random_access` 94.40 / 97.20.
- **Public API:** `key_lookup` is a `pub` field of `pub struct IndexData` (`index_reader/mod.rs:134`,
  reachable as `cqlite_core::storage::sstable::index_reader::IndexData`). Every field is `pub` and the
  struct is not `#[non_exhaustive]`, so changing the field's hasher *or adding a field* is an API
  change. **Internal route:** `lookup_partition` reads through `MaterializedIndex`
  (`index_reader/lazy.rs:36`, `pub(super)`), so a private second map with a fast hasher (`rustc-hash`
  is already a dependency) can sit there and leave `key_lookup` untouched, at the cost of a second
  map per open index. Task 11 Step 1 decides whether that memory cost is acceptable or whether this
  goes the proposal route (D4).

### H2: per-value type string parse (decode), not confirmed

- **Frames:** physical functions `CqlType::parse_with_depth` (28.02 / 25.48),
  `SSTableReader::decode_value_for_bench` (21.42 / 19.70; 17.25 of A is the inlined
  `Result<ComparatorType>` `?` after `ComparatorType::from_data_type`), `str::to_lowercase`
  (5.65 / 5.87), `vector_type::split_leading_identifier` (5.04 / 5.27), `ComparatorType::from_cql_type`
  (2.41 / 2.63), `str::trim_matches` (1.78 / 2.16).
- **Why not confirmed:** 64.31 vs 61.12 (Δ 3.19 pp). The drop is concentrated in the string-like
  cases: `type_text` 62.60 → 43.98, `type_ascii` 62.62 → 44.28, `type_blob` 65.14 → 49.43,
  `type_date` 76.06 → 56.89. In those cases B's allocator bucket is about twice A's (`type_text` 7.3
  → 16.0, `type_blob` 6.3 → 16.0, `type_date` 7.9 → 15.1). Both ran on core 5; B started with a
  15-minute load of 7.54 against A's 3.73. The single largest frame, `parse_with_depth`, also misses
  (Δ 2.54 pp). `decode_value_for_bench` alone passes (Δ 1.72 pp).
- **Source:** `value_parsing.rs:433`: `parse_value_with_schema_type` calls
  `ComparatorType::from_data_type(data_type)` (a full `CqlType::parse`) on every call. The decode
  bench reaches it through the `bench-internals` shim (`bench_shim.rs:24`). Live callers that parse
  per value or per row: `value_parsing.rs:397`, `row_cell_state_machine.rs:839`,
  `key_parsing.rs:330/380/419`. The read workload barely reaches it (`parse_with_depth` inclusive 0.0
  in A-cg read).
- **Targets:** 30 of 31 cases (all but `decode/vint_decode`), each ≥ 33% in both profiles.
- **Bench overhead:** 10.94 / 12.30 of decode is `Bencher::iter` itself, mostly
  `read_volatile::<Value>` (Criterion's `black_box` copy of the result). No fix can remove it.
- **Public API:** `ComparatorType::from_data_type` is `pub` (`types/comparator.rs:205`), but caching
  behind it or in `parse_value_with_schema_type` (`pub(in crate::storage::sstable::reader)`) changes
  no signature. Internal.

### H3: allocator (read)

- **Frames:** bucket `allocator` (`malloc` 12.37 / 12.49, `libc_malloc_internal_*`, `realloc`,
  `cfree`, `alloc::raw_vec`, …).
- **Call site: unknown.** In A-cg, 98.1% of samples whose leaf is in the allocator have no caller
  frame beyond the allocator: DWARF unwinding does not get through glibc's `malloc`. A rule needs a
  pattern and a site, so H3 cannot get a rule from this evidence. Finding the callers needs a
  different method (frame-pointer or LBR call stacks, or counting `malloc` callers with uprobes).
  That is a new measurement and needs the owner's approval.
- **Targets:** all 9 read cases (19.32 to 41.52 in A).

### H4: per-cell `Arc<str>` column-name refcount (read)

- **Frames:** `atomic_add::<usize>` (6.56 / 6.55; 6.21 of A is inlined in
  `parse_row_data_with_offset_impl`), `Arc<str>::drop` (3.62 / 3.58), `Arc<str>::clone`
  (1.00 / 1.09).
- **Source:** `row_decoder/row_data.rs:168, 656, 833` push `(Arc::clone(name), value)` per cell. The
  name is a schema-interned `Arc<str>` (issue #1334), so every cell does one atomic increment and
  later one atomic decrement on a shared counter.
- **Targets (A):** `clustering_slice` 34.53, `full_scan` 21.31, `scan_partition_dense_stream` 12.92,
  `clustering_range_bti` 7.20, `get_partition_big` 6.88, `point_lookup_repeated` 5.78,
  `get_partition_bti` 5.62, `clustering_point_bti` 4.61 (B within 0.3 pp). Not `type_heavy` (1.84).
- **Public API:** `ParsedRow` lives in `pub(crate) mod row_decoder`, but the names end up in
  `QueryRow::values: HashMap<Arc<str>, Value>` (`query/result.rs:78`, re-exported at `lib.rs:764`).
  Removing the refcount for returned rows changes a public type. An internal partial route (carry a
  column index and clone the name only when a row is materialised into a `QueryRow`) helps only if
  rows are decoded and then dropped. Whether they are, e.g. in `clustering_slice`, is not established.

### H5: memcpy (read)

- `__memcpy_ifunc_target` 4.97 / 5.06. Targets: `clustering_point_bti` 13.16, `clustering_range_bti`
  9.91, `get_partition_bti` 5.81, `point_lookup_repeated` 5.51, `get_partition_big` 4.61. Same problem
  as H3: no caller in A-cg, so no site.

### H6: `parse_clustering_prefix` (read)

- Physical function 3.35 / 3.11, of which 1.66 is inlined `[u8]` equality (`equal_same_length`).
  Targets: `clustering_point_bti` 16.57 / 14.99, `clustering_range_bti` 10.65 / 10.23. Internal
  (`pub(crate)` module). Small: the Amdahl bound on 2 cases is what Task 11 Step 1 will test against T.

## Round 1 hypotheses

From the brief, with spec 7.2's guessed sizes.

| hypothesis | guess | round 2 (A / B) | status |
|---|---|---|---|
| decode type parsing re-parses per value | ~55% of decode | H2: 64.31 / 61.12; `parse_with_depth` alone 28.02 / 25.48 | **changed**: larger than guessed, but not confirmed (Δ 3.19 pp) |
| SipHash | ~22% of partition_lookup | `core::hash::sip` frames only: 38.9 / 29.7, plus inlined `rotate_left` / `wrapping_add` 17.7 / 16.9; inclusive 25.6 (A-cg) | **confirmed as part of H1**; not stable as a frame set on its own (Δ 9 pp) |
| bcmp | ~20% of partition_lookup | `__bcmp_ifunc_target` 9.01 / 12.93 (plus `[u8]::eq` 3.76 / 6.59); read 0.93 / 0.90 | **changed**: part of H1 (hash-table key compare); not confirmed alone (Δ 3.9 pp) |
| allocator | ~31% of read | H3: 31.20 / 31.26; decode 8.1 / 9.6; partition_lookup 0.0 | **confirmed** (read); no call site |
| memcpy | 7% of read | H5: 5.04 / 5.14 | **confirmed**, smaller than guessed |
| `Arc<str>` clone/drop | ~10% of read | H4: 11.17 / 11.22 | **confirmed** |
| `Value::into_owned` | (named in the brief) | 2.61 / 2.52 (read) | **not confirmed** (< 3%) |

## Owner decisions (2026-11-17)

1. **H2:** a third decode profile C. H2 is confirmed iff C's H2 share is ≥ 3% and within 2 pp of
   A's (64.31) or B's (61.12). If C fails, H2 is skipped and not revisited (DECISIONS R22).
2. **H3 and H5:** a branch-stack (LBRv2) profile of the read binary, structural evidence only. If
   branch sampling does not work in this container, both are skipped. No frame-pointer rebuild
   (DECISIONS R23).
3. **H1:** the internal fix (a private fast-hash map in `MaterializedIndex`) counts toward the 4
   fixes, plus a proposal for a hasher change on `IndexData::key_lookup` (DECISIONS R24).
4. **Null runs:** the noise-gate soft cap. At most 3 further `NOISE_GATE_FAILED` nulls per workload,
   each started ≥ 2 h after the previous attempt on that workload; reaching it is stop-and-ask
   (DECISIONS R21). Only completed nulls with `b > 0.05` count toward the 3-try cap (DECISIONS R20).

## Profile C (H2) and branch stacks (H3/H5)

Added 2026-11-21 (close-out Task 6). Ledger: profile C `R2-000014` (decode, 2026-11-21 04:32Z, core 7),
branch-stack profile A-br `R2-000015` (read). This section supersedes the ranking table above where
they differ; the table itself is left as it was.

### H2: profile C verdict (DECISIONS R22)

H2 share = samples whose leaf physical symbol is one of R22's six functions, over all samples
(`round2/h2_group.py`, output in `h2-group.json`). The same script gives A 64.31 and B 61.12 from the
raw A and B scripts, matching the ranking table.

| physical function | A | B | C |
|---|---|---|---|
| `CqlType::parse_with_depth` | 28.02 | 25.48 | 37.78 |
| `SSTableReader::decode_value_for_bench` | 21.42 | 19.70 | 22.43 |
| `str::to_lowercase` | 5.65 | 5.87 | 3.73 |
| `vector_type::split_leading_identifier` | 5.04 | 5.27 | 2.32 |
| `ComparatorType::from_cql_type` | 2.41 | 2.63 | 2.21 |
| `str::trim_matches` | 1.78 | 2.16 | 1.17 |
| **H2 total** | **64.31** | **61.12** | **69.63** |
| samples | 3,115,056 | 3,196,030 | 3,081,892 |

R22's test, as written:

- C ≥ 3%: 69.63 ≥ 3, **yes**.
- |C − 64.31| = |69.63 − 64.31| = 5.32 pp; < 2? **no**.
- |C − 61.12| = |69.63 − 61.12| = 8.51 pp; < 2? **no**.

**Verdict: H2 is not confirmed. Per R22 it is skipped and not revisited.**

C is higher than both earlier profiles, not between them. The rise is in one member:
`CqlType::parse_with_depth` is 37.78 in C against 28.02 (A) and 25.48 (B); `str::to_lowercase` and
`vector_type::split_leading_identifier` are lower in C than in A or B. The three profiles span
8.51 pp.

Conditions that differ, recorded next to the verdict and **not used to change it**:

| | A | B | C |
|---|---|---|---|
| kernel | 7.0.0-42 | 7.0.0-42 | 7.0.0-43 (host rebooted 2026-11-19, R26) |
| core | 5 | 5 | 7 (chosen by `profile.sh`, as R22 requires) |
| perf | 7.0.14 | 7.0.14 | 7.0.14 (same binary, R26) |
| load at start (1/5/15 min) | 0.87/1.20/3.73 | 1.99/1.71/7.54 | 0.47/0.43/0.55 |

R26 said the 2 pp band would have to absorb the kernel change, and R17 found that cores differ. Either
may explain part of the gap; this evidence cannot separate them from a real instability in the H2
share, and R22 has no exception for them.

Target set under R22's rule (cases with H2 share ≥ 3% in A, B and C): 30 of 31, all but
`decode/vint_decode`. It is recorded for completeness; with H2 skipped, no fix will use it.

| case | A | B | C | ≥ 3% in all |
|---|---|---|---|---|
| `decode/text_heavy` | 60.60 | 68.65 | 72.44 | yes |
| `decode/type_ascii` | 62.62 | 44.28 | 65.39 | yes |
| `decode/type_bigint` | 77.19 | 79.03 | 82.08 | yes |
| `decode/type_blob` | 65.14 | 49.43 | 66.64 | yes |
| `decode/type_boolean` | 80.18 | 76.80 | 82.56 | yes |
| `decode/type_counter` | 76.94 | 77.35 | 85.77 | yes |
| `decode/type_date` | 76.06 | 56.89 | 80.01 | yes |
| `decode/type_decimal` | 70.69 | 71.41 | 76.49 | yes |
| `decode/type_double` | 79.49 | 78.14 | 82.13 | yes |
| `decode/type_duration` | 75.07 | 75.71 | 80.63 | yes |
| `decode/type_float` | 76.46 | 77.46 | 81.22 | yes |
| `decode/type_frozen` | 33.51 | 34.34 | 36.97 | yes |
| `decode/type_inet` | 60.34 | 60.64 | 65.83 | yes |
| `decode/type_int` | 73.33 | 73.80 | 79.10 | yes |
| `decode/type_json` | 37.32 | 38.09 | 42.77 | yes |
| `decode/type_list` | 54.78 | 54.05 | 58.98 | yes |
| `decode/type_map` | 42.24 | 41.73 | 49.50 | yes |
| `decode/type_set` | 56.51 | 55.79 | 64.74 | yes |
| `decode/type_smallint` | 76.91 | 76.43 | 82.10 | yes |
| `decode/type_text` | 62.60 | 43.98 | 66.84 | yes |
| `decode/type_time` | 71.35 | 64.84 | 76.39 | yes |
| `decode/type_timestamp` | 78.29 | 67.08 | 78.48 | yes |
| `decode/type_timeuuid` | 75.65 | 76.09 | 86.42 | yes |
| `decode/type_tinyint` | 75.94 | 79.85 | 82.46 | yes |
| `decode/type_tuple` | 52.84 | 57.86 | 64.67 | yes |
| `decode/type_udt` | 57.61 | 59.78 | 62.22 | yes |
| `decode/type_uuid` | 75.00 | 71.46 | 81.01 | yes |
| `decode/type_varchar` | 64.24 | 48.66 | 69.88 | yes |
| `decode/type_varint` | 67.91 | 66.17 | 66.84 | yes |
| `decode/vint_decode` | 0.01 | 0.01 | 0.01 | no |
| `decode/wide_row_primitives` | 79.06 | 82.57 | 89.66 | yes |

### H3/H5: call sites from branch stacks (DECISIONS R23, R27)

`perf record -e cycles:u -F 999 -j any_call,u` on the read binary, attributed by `callers.py`
(`A-br/read.json`). Structural evidence only: no share in the ranking depends on it. 178,433 samples;
`brstack_empty_pct` 0.03% (samples with no branch records).

| bucket | bucket samples | top-25 sites | `other_sites_pct` | `no_caller_pct` | `inconsistent_pct` |
|---|---|---|---|---|---|
| allocator | 56,856 | 52.70 | 14.26 | 4.31 | 28.73 |
| memcpy | 9,663 | 65.57 | 10.96 | 0.02 | 23.45 |

Each row sums to 100% of the bucket's samples that carry branch records (R27). `inconsistent` samples
are those where a newer call into ordinary bench code follows the candidate call (sampling skid); they
are credited to no site, so every site share below is a lower bound.

**H3 allocator: top 10 call sites**

| # | site | share of bucket | share of workload |
|---|---|---|---|
| 1 | `<cqlite_core::query::engine::QueryEngine>::execute::{closure#0}::{closure#0} (cqlite-core/src/query/engine/deadline.rs:303)` | 8.22 | 2.62 |
| 2 | `alloc::str::convert_while_ascii (library/alloc/src/str.rs:822)` | 7.62 | 2.43 |
| 3 | `<u8 as <[_]>::to_vec_in::ConvertVec>::to_vec::<alloc::alloc::Global> (library/alloc/src/slice.rs:448)` | 6.60 | 2.10 |
| 4 | `core::ptr::drop_glue::<cqlite_core::schema::Column> (library/core/src/ptr/mod.rs:825)` | 4.40 | 1.40 |
| 5 | `<cqlite_core::schema::Column as core::clone::Clone>::clone (cqlite-core/src/schema/mod.rs:239)` | 2.64 | 0.84 |
| 6 | `cqlite_core::query::select_executor::row_build::build_row_from_scan_cached (cqlite-core/src/query/select_executor/row_build.rs:246)` | 2.51 | 0.80 |
| 7 | `<hashbrown::raw::RawTableInner>::free_buckets::<alloc::alloc::Global> (/rust/deps/hashbrown-0.17.1/src/raw.rs:3114)` | 2.27 | 0.72 |
| 8 | `<cqlite_core::schema::Column as core::clone::Clone>::clone (cqlite-core/src/schema/mod.rs:243)` | 2.19 | 0.70 |
| 9 | `<cqlite_core::storage::sstable::reader::parsing::row_decoder::V5CompressedLegacyParser>::parse_row_data_with_offset_impl (cqlite-core/src/storage/sstable/reader/parsing/row_decoder/row_data.rs:75)` | 1.88 | 0.60 |
| 10 | `<cqlite_core::storage::sstable::reader::parsing::row_decoder::RowColumnResolution>::build::{closure#0}::{closure#1} (cqlite-core/src/storage/sstable/reader/parsing/row_decoder/mod.rs:218)` | 1.87 | 0.59 |

No site reaches 25% of the bucket (largest: 8.22%). The cost is spread out. **No rule.**

**H5 memcpy: top 10 call sites**

| # | site | share of bucket | share of workload |
|---|---|---|---|
| 1 | `<cqlite_core::storage::sstable::reader::types::SSTableReader>::bti_pull_decompressed_chunk::{closure#0} (cqlite-core/src/storage/sstable/reader/data_access/bti_point.rs:787)` | 36.25 | 1.96 |
| 2 | `<cqlite_core::query::engine::QueryEngine>::execute::{closure#0}::{closure#0} (cqlite-core/src/query/engine/deadline.rs:303)` | 6.82 | 0.37 |
| 3 | `<cqlite_core::query::select_executor::SelectExecutor>::execute::{closure#0} (cqlite-core/src/query/select_executor/execute.rs:44)` | 3.13 | 0.17 |
| 4 | `core::ptr::copy_nonoverlapping::<u8> (library/core/src/ptr/mod.rs:551)` | 2.19 | 0.12 |
| 5 | `<cqlite_core::query::engine::QueryEngine>::execute::{closure#0} (cqlite-core/src/query/engine/deadline.rs:293)` | 1.71 | 0.09 |
| 6 | `<cqlite_core::storage::sstable::reader::parsing::row_decoder::V5CompressedLegacyParser>::parse_row_data_with_offset_impl (cqlite-core/src/storage/sstable/reader/parsing/row_decoder/row_data.rs:833)` | 1.25 | 0.07 |
| 7 | `cqlite_core::storage::sstable::compression::snappy_decompress_raw (cqlite-core/src/storage/sstable/compression.rs:148)` | 1.25 | 0.07 |
| 8 | `<cqlite_core::query::engine::QueryEngine>::execute_select_query::{closure#0} (cqlite-core/src/query/engine.rs:376)` | 1.20 | 0.07 |
| 9 | `<cqlite_core::schema::Column as core::clone::Clone>::clone (cqlite-core/src/schema/mod.rs:243)` | 1.15 | 0.06 |
| 10 | `<u8 as <[_]>::to_vec_in::ConvertVec>::to_vec::<alloc::alloc::Global> (library/alloc/src/slice.rs:448)` | 0.96 | 0.05 |

A site reaches ≥ 25% of the bucket: `<cqlite_core::storage::sstable::reader::types::SSTableReader>::bti_pull_decompressed_chunk::{closure#0} (cqlite-core/src/storage/sstable/reader/data_access/bti_point.rs:787)` at 36.25%. **Rule candidate.**

The H5 site is concentrated in two cases: it is 73.0% of the memcpy bucket in
`read/clustering_point_bti` and 65.5% in `read/clustering_range_bti` (`per_case_sites` in
`A-br/read.json`), the two cases where H5's self share is largest (13.16 and 9.91 in A). Over the whole
read workload the site carries 1.96% of A-br's samples, below the 3% target-case floor as a workload
figure; whether a fix clears T on its target cases is Phase 3's Amdahl check (Task 11 Step 1).

### Phase 3 candidates after this section

| rank | hot spot | state | route |
|---|---|---|---|
| 1 | H1 `key_lookup` HashMap get (partition_lookup) | confirmed | internal fix + proposal (R24) |
| — | H2 type string re-parse (decode) | **skipped, not revisited** (R22; C 69.63 vs 64.31 / 61.12) | — |
| — | H3 allocator (read) | confirmed share, **no rule**: no call site ≥ 25% of the bucket | — |
| 2 | H4 per-cell `Arc<str>` refcount (read) | confirmed | public type at the end of the path; internal partial route not established |
| 3 | H5 memcpy (read) | confirmed share; rule candidate site `bti_point.rs:787` (36.25% of bucket) | to be read in Phase 3 |
| 4 | H6 `parse_clustering_prefix` (read) | confirmed | internal, small |
