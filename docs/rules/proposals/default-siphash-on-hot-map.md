# Proposal: fast hasher on `IndexData::key_lookup` (public-API change)

Proposal, not a fix. Not made in round 2 (spec section 8); recorded per DECISIONS R24. The internal
route for the same hot spot is `docs/rules/default-siphash-on-hot-map.md`, which holds the pattern,
detection, evidence and target cases.

## Proposed change

Change the field's type in `cqlite_core::storage::sstable::index_reader::IndexData`:

```rust
// now
pub key_lookup: HashMap<Arc<[u8]>, usize>,
// proposed
pub key_lookup: HashMap<Arc<[u8]>, usize, BuildHasherDefault<FxHasher>>,
```

(`rustc-hash` is already a dependency of `cqlite-core`.)

- **Why over the internal route:** one map instead of two per materialized index. The internal
  route keeps the SipHash map only so the public field stays as it is.
- **Who is affected:** any code that names the field's type, builds an `IndexData` by struct literal
  (every field is `pub` and the struct is not `#[non_exhaustive]`), or passes the map to something
  that expects the default hasher. `get`, `iter`, `len` and `insert` calls keep compiling. In the
  CQLite tree, `index_reader/parse.rs:76` builds the map; the tests in
  `tests/index_reader_memory_optimization_tests.rs` build their own local maps and are not affected.
- **Migration:** a semver-minor break for a 0.x crate; construct with `HashMap::default()` or
  `with_capacity_and_hasher`. Marking `IndexData` `#[non_exhaustive]` at the same time would make
  later changes of this kind non-breaking.
- **Risk:** FxHash is not keyed, so an SSTable written with keys chosen to collide could slow
  lookups on that file. SipHash's flooding resistance is lost for this map.

## Predicted gain

Same as the internal route (ratio 0.46 to 0.72 on the six partition_lookup cases), since the lookup
path is the same; the difference is memory and build time for the second map, which the
partition_lookup bench does not measure.

## Sizing

Not measured. A sizing run on `r2-proposal/default-siphash-on-hot-map` happens only after every fix
branch's confirmation and cross-check runs (spec 7.1), labelled "proposal, not a fix".

(Written before the sizing run. The result is appended below.)

### Sizing result, 2026-11-23 (proposal, not a fix)

Branch `r2-proposal/default-siphash-on-hot-map` at `a9878039`, cut from the base `7f9abe36`: the
field's type changed as above and the one constructor (`index_reader/parse.rs:76`) adapted; two
files, 5 lines added, 2 removed. No other public type had to change, and the rest of the workspace
compiled without edits.

| | value | record |
|---|---|---|
| sizing try 1 | `NOISE_GATE_FAILED` (4 sessions rejected for load 4.06 to 6.64; 16 of 20 completed) | R2-000048 |
| **sizing try 2, targets (= whole workload)** | ratio **0.4459**, 95% interval 0.4428 to 0.4493; b 0.0416, T 0.0832; 20 sessions, 1 rejected and replaced; no REGRESSION flag | R2-000049 |

Per-case ratios: `borrow_trait_lookup` 0.347, `position_0` 0.355, `hot_keys` 0.469,
`sequential_access` 0.484, `random_access` 0.529, `1000_lookups` 0.532.

- The record's verdict field reads WON because the referee applies the same rule to every run kind.
  This is a sizing run: it is not a round 2 win and the change has not been made.
- **Prediction held: no.** 0.4459 is below the predicted range (0.46 to 0.72), as with the internal
  route.
- **Against the internal route** (0.4419, R2-000031): the two ratios differ by less than T, so
  neither route is shown to be faster than the other on this bench. The case for the public route
  is one map instead of two, which the bench does not measure.
- Tested only by the `index_reader` tests (30 passed; 2 passed in
  `index_reader_memory_optimization_tests`) and a workspace `cargo check`. The full suite was not
  run on this branch, and no cross-check runs were made.
