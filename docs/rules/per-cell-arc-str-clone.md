# Rule: per-cell-arc-str-clone

Hot spot H4 (`round2/results/profile/HOTSPOTS.md`). Pre-registered before any code.
**Status: not attempted as a fix. Predicted gain of the internal route is below T; the public route
is a proposal (`docs/rules/proposals/per-cell-arc-str-clone.md`).**

## Pattern

A decoder attaches a reference-counted name (`Arc<str>`) to every decoded cell by cloning a shared,
schema-interned `Arc`. Each cell then costs one atomic increment on a counter shared by every row
and thread, and one atomic decrement when the row is dropped.

Here: `reader/parsing/row_decoder/row_data.rs:168, 656, 833` push `(Arc::clone(name), value)` into
`RowCells = Vec<(Arc<str>, Value)>`.

## Detection

- **Source:** `Arc::clone` (or `.clone()` on an `Arc`) inside a per-cell or per-row loop, where the
  `Arc` comes from a schema or other long-lived table and the clone is stored in the output element.
- **MIR:** `<Arc<T> as Clone>::clone` in a loop body whose result is moved into a pushed tuple or
  struct.
- **Machine code:** `lock incq` / `lock decq` on the same address from inside the decode loop;
  symbols `atomic_add::<usize>`, `<Arc<str> as Drop>::drop`.

## Evidence

Profiles R2-000011 (A), R2-000012 (B).

- Group self share of read: **11.17% (A) / 11.22% (B)**, Δ 0.04 pp, confirmed. Members:
  `atomic_add::<usize>` 6.56 / 6.55, `Arc<str>::drop` 3.62 / 3.58, `Arc<str>::clone` 1.00 / 1.09.
- Code read at `7f9abe36` (2026-11-23, no build): on every read bench path a returned cell costs
  exactly one increment/decrement pair. Nothing re-clones; sort, merge, cap and row build all move.

## Target cases

Eight of nine read cases (share ≥ 3% in A; B within 0.3 pp). Not `read/type_heavy` (1.84). Also in
`round2/results/targets/per-cell-arc-str-clone.json`.

| case | share A | cells that reach the caller |
|---|---|---|
| `read/clustering_slice` | 34.53 | all cells of returned rows (`SELECT * LIMIT 64`); rows decoded ahead of the limit are dropped |
| `read/full_scan` | 21.31 | all |
| `read/scan_partition_dense_stream` | 12.92 | all |
| `read/clustering_range_bti` | 7.20 | rows inside the range; about half the decoded rows are rejected (inferred from block size) |
| `read/get_partition_big` | 6.88 | 1 of 18 regular columns (projection) |
| `read/point_lookup_repeated` | 5.78 | 1 of 5 |
| `read/get_partition_bti` | 5.62 | 1 of 5 |
| `read/clustering_point_bti` | 4.61 | 1 row of about 8 decoded (inferred from block size) |

Null: R2-000016. `r2 nullband --null R2-000016 --workload read --targets …`:
**b_targets = 0.0160, b_workload = 0.0145, T = 0.05.**

## Public API

The carrier types are public: `RowCells` and `ScanRow` (`types.rs:205, 220`, re-exported from the
crate root) appear in the signatures of `StorageEngine::scan`, `scan_stream*`,
`scan_partition_clustering`, `SSTableManager::scan`, `SSTableReader::scan` and
`build_row_from_scan[_cached]`, and the names end in `QueryRow::values: HashMap<Arc<str>, Value>`
(`query/result.rs:78`). For a cell that reaches the caller, the one increment/decrement pair is the
minimum these types allow. Removing it changes a public type: proposal.

**Internal route:** skip the name clone for cells that never reach the caller (columns outside the
projection; rows rejected by the clustering filter). It cannot touch cells that are returned.

## Predicted gain (internal route)

Ratio per case ≈ `1 − s·f`, `s` = share above, `f` = fraction of the case's cells that are dropped
before the caller. The verdict is the geometric mean over the eight targets.

| case | s | f (high) | ratio (high f) |
|---|---|---|---|
| `clustering_slice` | 0.3453 | 0 (the dropped rows sit in public `ScanRow` channels) | 1.000 |
| `full_scan` | 0.2131 | 0 | 1.000 |
| `scan_partition_dense_stream` | 0.1292 | 0 | 1.000 |
| `clustering_range_bti` | 0.0720 | 0.50 | 0.964 |
| `get_partition_big` | 0.0688 | 0.94 | 0.935 |
| `point_lookup_repeated` | 0.0578 | 0.80 | 0.954 |
| `get_partition_bti` | 0.0562 | 0.80 | 0.955 |
| `clustering_point_bti` | 0.0461 | 0.85 | 0.961 |

Geometric mean at high `f`: exp((ln 0.964 + ln 0.935 + ln 0.954 + ln 0.955 + ln 0.961) / 8) =
exp(−0.237 / 8) = **0.971**. At half those fractions: **0.985**.
**Predicted ratio range: 0.971 to 0.985.**

`1 − T` = 0.950. The whole range is above it: **not attempted: predicted gain below T.**

This counts only the refcount cost, which is what the profile ranked. A change that skips decoding
rejected rows or unselected columns altogether would save more than the refcount, but that is a
different pattern ("decode, then discard") that the profile did not rank as a hot spot; it is not
attempted under this rule.

## Outcome

Not attempted (2026-11-23): predicted gain of the internal route below T; public route recorded as
a proposal. Does not count toward the 4 fixes.
