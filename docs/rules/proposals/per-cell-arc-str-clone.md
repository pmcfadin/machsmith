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
