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
