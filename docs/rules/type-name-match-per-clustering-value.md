# Rule: type-name-match-per-clustering-value

Hot spot H6 (`round2/results/profile/HOTSPOTS.md`). Pre-registered: this file is committed before
branch `r2/type-name-match-per-clustering-value` has any code.

## Pattern

A decoder picks the decode routine for each value by lower-casing the column's type name into a new
`String` and comparing it against string literals, once per value, although the type is fixed for
the whole table. Each value pays an allocation, a copy, several byte-string compares and a free.

Here: `parse_clustering_prefix` (`reader/parsing/row_decoder/row_framing.rs:1276`, `pub(super)`)
calls `parse_clustering_value` (line ~1396, private) per clustering column per row, which does
`col.data_type.to_lowercase()` and then `match normalized.as_str() { "timestamp" | … }`.

## Detection

- **Source:** `to_lowercase()` / `to_uppercase()` / `format!` on a schema string, followed by a
  `match` on `.as_str()` with string-literal arms, inside a function called per row or per value.
- **MIR:** `<str>::to_lowercase` followed by a chain of `<[u8] as PartialEq>::eq` (or `bcmp`) calls
  against constants in a function reached from a decode loop.
- **Machine code:** inlined `equal_same_length` / `bcmp` against `.rodata` strings and a
  `malloc`/`free` pair per call, in a function that otherwise only loads a few fixed-width bytes.

## Evidence

Profiles R2-000011 (A), R2-000012 (B).

- Physical function `parse_clustering_prefix`: **3.35% (A) / 3.11% (B)** of read, Δ 0.25 pp,
  confirmed. Of A's 3.35, 1.66 is inlined `[u8]` equality (`equal_same_length`).
- The lower-casing's allocation and free are counted in the allocator bucket (H3), not here.

## Target cases

`read/clustering_point_bti` (16.57 A / 14.99 B) and `read/clustering_range_bti` (10.65 / 10.23).
No other read case reaches 3%. Also in
`round2/results/targets/type-name-match-per-clustering-value.json`.

Null: R2-000016. `r2 nullband --null R2-000016 --workload read --targets …`:
**b_targets = 0.0441, b_workload = 0.0145, T = 0.0881** (same two cases as
`copy-cached-chunk-into-window`).

## Predicted gain

Ratio per case ≈ `1 − s·f`; the verdict is the geometric mean of the two cases.

- Low: `s` = the lower of A and B (0.1499, 0.1023); `f` = 0.5 (only the string compares go, which
  are 1.66 / 3.35 of the function). Ratios 0.925 and 0.949, geometric mean **0.937**.
- High: `s` = the higher (0.1657, 0.1065); `f` = 0.8 (compares and lower-casing go; the vint read
  and the value load stay). Ratios 0.867 and 0.915, geometric mean **0.891**.
- **Predicted ratio range: 0.891 to 0.937.**

`1 − T` = 0.912. The range is not entirely above it, so the rule is attempted. Most of the range is
inside the band: a TIED or INCONCLUSIVE verdict is a likely outcome.

Not predicted: the allocator time of the per-value `String` (in H3's bucket).

## Public API

Internal. Both functions are in `pub(crate) mod row_decoder`. `ClusteringColumn::data_type` is a
`pub` field and must keep its type; the fix may not add a field to a public schema type. Decode
results must be identical for every type name the current match accepts, including the fallback
arm, with no inference from bytes (CQLite's no-heuristics rule).

## Outcome

(appended after the confirmation run)
