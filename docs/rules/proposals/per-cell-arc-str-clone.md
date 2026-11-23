# Proposal: stop carrying an `Arc<str>` name per cell (public-API change)

Proposal, not a fix. Not made in round 2 (spec section 8). Pattern, detection, evidence and target
cases are in `docs/rules/per-cell-arc-str-clone.md`.

## Proposed change

Carry column identity per row instead of per cell. One shape:

- `RowCells` becomes a struct holding `Arc<[Arc<str>]>` (the table's column names, one refcount per
  row) and `Vec<(u32, Value)>` (column index, value), instead of `Vec<(Arc<str>, Value)>`.
- `QueryRow::values` (`HashMap<Arc<str>, Value>`) either keeps its type and pays the clone only at
  the query boundary, or moves to the same indexed shape with a by-name accessor.

- **Who is affected:** everything that names `RowCells`, matches on `ScanRow::Row(cells)`, or
  iterates `(name, value)` pairs: the public `StorageEngine::scan*`, `SSTableManager::scan`,
  `SSTableReader::scan`, `build_row_from_scan[_cached]`, the doc-hidden
  `V5CompressedLegacyParser::parse_block*`, and the Python and Node bindings if they read
  `QueryRow::values` directly.
- **Migration:** an iterator `cells.iter_named() -> impl Iterator<Item = (&Arc<str>, &Value)>` keeps
  most call sites to a one-line change. If `QueryRow::values` keeps its type, binding users see no
  change and the gain is limited to cells dropped before the query boundary plus the scan APIs.
- **Risk:** large mechanical diff across the read path; the indexed shape needs a rule for static
  and clustering-key cells, which today are pushed with their own names.

## Predicted gain

If the per-cell pair is removed for every cell (both types change): ratio per case ≈ `1 − s·f`
with `f` from 0.8 to 1.0 (a per-row refcount remains). Geometric mean over the eight targets with
the shares in the rule entry: `f` = 1.0 gives exp(−1.112 / 8) = 0.870; `f` = 0.8 gives
exp(−0.866 / 8) = 0.897. **Predicted ratio range: 0.87 to 0.90.**

## Sizing

Not measured. A sizing run on `r2-proposal/per-cell-arc-str-clone`, if the owner wants one, happens
only after every fix branch's confirmation and cross-check runs (spec 7.1), labelled "proposal, not
a fix".

(Written before the sizing run. The owner asked for one on 2026-11-23; the result is appended below.)

### Sizing result, 2026-11-23 (proposal, not a fix)

Branch `r2-proposal/per-cell-arc-str-clone` at `806bd8b9`, cut from the base `7f9abe36`. Both parts
of the proposal were built:

- `RowCells` is a struct holding a shared name table (`ColumnNames = Arc<[Arc<str>]>`) and
  `Vec<(u32, Value)>`; `iter_named()` yields `(&Arc<str>, &Value)`.
- `QueryRow::values` is a new `RowValues` type: still a SipHash hash table, but of `(u32, Value)`
  entries indexing the shared table, with `HashMap`-like accessors by name.
- One name table per reader, built on the first row resolution and reused while the layout matches
  name for name; static, partition-key and clustering-key cells take slots in the same table.
- Diff: 71 files, 1,256 lines added, 292 removed. `benches/read.rs` compiles unchanged.

Every signature that names `RowCells` changed (`ScanRow::Row`, `StorageEngine::scan*`,
`SSTableManager::scan`, `SSTableReader::scan`, `build_row_from_scan[_cached]`, the doc-hidden
`V5CompressedLegacyParser::parse_block*`), and `QueryRow::values` changed type. `RowValues` has no
`entry` method; the one caller in `cqlite-flight` was rewritten.

| | value | record |
|---|---|---|
| sizing tries 1 and 2 | `NOISE_GATE_FAILED` (load from other work on the host) | R2-000050, R2-000051 |
| **sizing try 3, targets** | **TIED**: ratio **0.9826**, 95% interval 0.9750 to 0.9907; b 0.0160, T 0.05; 20 sessions, 0 rejected; no REGRESSION flag | R2-000052 |
| sizing try 3, whole workload | TIED: 0.9856 (0.9774 to 0.9956) | R2-000052 |

Per-case point ratios (R2-000052): `scan_partition_dense_stream` 0.926, `clustering_slice` 0.937,
`full_scan` 0.980, `get_partition_big` 0.991, `clustering_point_bti` 1.000, `get_partition_bti`
1.007, `point_lookup_repeated` 1.009, `clustering_range_bti` 1.015; `type_heavy` (not a target)
1.009.

- **Prediction held: no.** 0.9826 is far above the predicted range (0.87 to 0.90). On the
  pre-registered statistic the change is inside the noise threshold, so no gain is claimed.
- The two scan cases with the largest profile share moved the most, but by much less than their
  share: `clustering_slice` had 34.53% of its samples in the refcount group and measured 0.937.
  The verdict is on the eight-case set; a per-case ratio is not a claim.
- **Why the prediction was wrong is not established.** Candidates, none tested: the profile share
  overstates what removing the refcount saves (the samples sit on the atomic instructions, but the
  time may be a cache miss on the name that the next reader of the name pays anyway); the new shape
  adds cost of its own (one more indirection per hash probe, a larger `QueryRow`, a name compare per
  resolution build, one refcount per row on a counter shared by the whole reader); the point reads
  return one row, so they have little to save.
- **How the branch was checked.** A fresh reviewer read the diff for correctness and for fairness
  of the measurement. The first version rebuilt the name table once per partition, which on a
  one-row-per-partition table moves the refcount work instead of removing it; that was fixed before
  any sizing run of this branch (tries 1 to 3 all used `806bd8b9`). A probe (scratch code, not on
  the branch) then counted zero cache misses in the timed query of all eight cases.
- Tests: `cqlite-core` suite 6,429 passed, 12 failed, all on the base list (run at `1f121b31`; the
  later commit changes one test file). Workspace suite run only at the first version (9,623 passed,
  16 failed, all on the base list). Not recorded in the ledger; no cross-check runs were made.
- The branch is a measurement vehicle, not a design: cold paths (merge, compaction, export, CLI)
  now clone names where they did not before, and owned iteration of `RowValues` allocates.

