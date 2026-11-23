# Rule: copy-cached-chunk-into-window

Hot spot H5 (`round2/results/profile/HOTSPOTS.md`). Pre-registered: this file is committed before
branch `r2/copy-cached-chunk-into-window` has any code.

## Pattern

A read path fetches an immutable, reference-counted buffer from a cache and then copies all of it
into a fresh `Vec<u8>` so that a parser can read one contiguous slice. When one cached buffer
already covers the whole range the parser needs, the copy (and the `Vec` growth behind it) is pure
overhead on every request, including warm cache hits.

Here: `SSTableReader::bti_pull_decompressed_chunk`
(`reader/data_access/bti_point.rs:746`, private) gets a decompressed chunk from
`ChunkSource::chunk` (the shared chunk cache) and does
`window.extend_from_slice(&decompressed_chunk)` at line 787. Its callers (lines 632 and 668) build
`window` for one partition and parse it once.

## Detection

- **Source:** `extend_from_slice` / `to_vec` / `copy_from_slice` whose source is a value just
  returned by a cache `get` (an `Arc<[u8]>`, `Bytes` or similar), into a buffer that is only read
  afterwards.
- **MIR:** a call to a cache accessor returning a shared buffer, followed in the same block by
  `Vec::<u8>::extend_from_slice` on its deref, with no later mutable use of the `Vec`.
- **Machine code:** `memcpy` reached from the function, with a length equal to the cache's block
  size (a branch-stack profile shows the call site; frame-pointer or DWARF unwinding does not,
  see HOTSPOTS.md).

## Evidence

- Flat profiles R2-000011 (A) / R2-000012 (B): `__memcpy_ifunc_target` is 4.97 / 5.06% of read
  (H5 5.04 / 5.14, confirmed). By case in A: `clustering_point_bti` 13.16, `clustering_range_bti`
  9.91, `get_partition_bti` 5.81, `point_lookup_repeated` 5.51, `get_partition_big` 4.61.
- Branch-stack profile R2-000015 (A-br, structural only): this call site is 36.25% of the memcpy
  bucket (1.96% of read). By case: 72.97% of the bucket and **9.87% of the case** in
  `clustering_point_bti`; 65.46% of the bucket and **6.50% of the case** in
  `clustering_range_bti`; 0.89% of the case in `get_partition_big`; not among the listed sites of
  the other cases. Site shares are lower bounds: 23.45% of memcpy samples are `inconsistent` and
  credited to no site (DECISIONS R27).

## Target cases

`read/clustering_point_bti` and `read/clustering_range_bti`: the two cases where this site is
≥ 3% of the case (DECISIONS R30). Both are also H5 targets in the flat profiles. Also in
`round2/results/targets/copy-cached-chunk-into-window.json`.

Null: R2-000016. `r2 nullband --null R2-000016 --workload read --targets …`:
**b_targets = 0.0441, b_workload = 0.0145, T = 0.0881.**

## Predicted gain

Ratio per case ≈ `1 − s·f`; the verdict is the geometric mean of the two cases.

- Low: `s` as measured (0.0987, 0.0650), `f` = 0.8 (a partition that spans more than one chunk
  still needs a stitched copy). Ratios 0.921 and 0.948, geometric mean **0.934**.
- High: `s` scaled by 1 / (1 − 0.2345) for the uncredited samples (0.1289, 0.0849; both stay below
  the cases' flat memcpy shares), `f` = 1.0. Ratios 0.871 and 0.915, geometric mean **0.893**.
- **Predicted ratio range: 0.893 to 0.934.**

`1 − T` = 0.912. The range is not entirely above it, so the rule is attempted. Most of the range is
inside the band: a TIED or INCONCLUSIVE verdict is a likely outcome.

Not predicted: any saving from the `Vec` allocation that the copy grows into.

## Public API

Internal. `bti_pull_decompressed_chunk` and its callers are private methods of `SSTableReader` in
`reader/data_access/bti_point.rs`. The fix must not change `ChunkSource`'s or the chunk cache's
public items, and must keep `work_counters::add_chunk_decompressed` counting as it does now.

## Outcome

(appended after the confirmation run)
