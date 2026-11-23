# Rule: default-siphash-on-hot-map

Hot spot H1 (`round2/results/profile/HOTSPOTS.md`). Pre-registered: this file is committed before
branch `r2/default-siphash-on-hot-map` has any code.

## Pattern

A `std::collections::HashMap` with the default `RandomState` hasher (SipHash-1-3) sits on a lookup
path that is called per request. The keys are short byte strings that do not come from an attacker
who can choose them to collide (here: partition key bytes already stored in the SSTable's own
`Index.db`). Each lookup pays the keyed SipHash rounds, then the probe, then one key compare.

Here: `IndexReader::lookup_partition` (`index_reader/mod.rs:283`) calls
`key_lookup.get(key_digest)` on `HashMap<Arc<[u8]>, usize>`.

## Detection

- **Source:** a `HashMap<K, V>` (two type parameters, or `HashMap::new` / `with_capacity`) whose
  `get` / `contains_key` is reached from a function the profile shows at ≥ 3% of a workload, with `K`
  a byte slice, string or integer.
- **MIR:** a call to `<RandomState as BuildHasher>::hash_one` or
  `<DefaultHasher as Hasher>::write` in a function on the hot path.
- **Machine code:** the symbol `<core::hash::sip::Hasher<Sip13Rounds> as Hasher>::write`, or inlined
  SipHash rounds (the `rotate_left` 13/16/17/21/32 sequence) inside the hot physical function.

## Evidence

Profiles R2-000011 (A), R2-000012 (B), R2-000013 (A-cg, structural only).

- Group self share of partition_lookup: **95.96% (A) / 97.22% (B)**, Δ 1.27 pp, confirmed.
  Members: `IndexReader::lookup_partition` 68.98 / 69.89 (with `HashMap::get`, the hashbrown probe
  and most SipHash rounds inlined), `Hasher<Sip13Rounds>::write` 17.97 / 14.40,
  `__bcmp_ifunc_target` 9.01 / 12.93.
- Inclusive (A-cg): `lookup_partition` 73.9, of which SipHash 25.6 and bcmp 33.0.
- By inlined frame: `core::hash::sip` frames 38.9 / 29.7, plus inlined `rotate_left` /
  `wrapping_add` 17.7 / 16.9. So hashing is about 47–57% of the workload's samples; the split
  between hashing and compare is not stable between A and B, the group total is.

## Target cases

All six cases of partition_lookup (share ≥ 3% in A and B). Also in
`round2/results/targets/default-siphash-on-hot-map.json`.

| case | A | B |
|---|---|---|
| `index_lookup_cold/position_0` | 98.61 | 98.17 |
| `index_lookup_warm/hot_keys` | 96.63 | 97.46 |
| `cache_operations/borrow_trait_lookup` | 96.46 | 98.20 |
| `lookup_throughput/1000_lookups` | 95.39 | 97.36 |
| `lookup_distribution/sequential_access` | 94.43 | 94.99 |
| `lookup_distribution/random_access` | 94.40 | 97.20 |

Null: R2-000022. `r2 nullband --null R2-000022 --workload partition_lookup --targets …`:
**b = 0.0416, T = 0.0832** (the target set is the whole workload, so `b_targets = b_workload`).

## Predicted gain

Ratio ≈ `1 − s·f`, with `s` the group's share of target time and `f` the fraction of the group the
fix removes.

- `s` = 0.96 (the lower of A and B, rounded).
- The fix removes hashing cost, not the probe or the key compare. Hashing is 47–57% of samples, so
  45–59% of the group. A multiply-rotate hasher (`rustc-hash`) still costs something per key, so
  assume it removes 65–95% of the hashing cost.
- Low `f` = 0.45 × 0.65 = 0.29. High `f` = 0.59 × 0.95 = 0.56.
- **Predicted ratio range: 0.46 to 0.72** (1 − 0.96 × 0.56 = 0.46; 1 − 0.96 × 0.29 = 0.72).

`1 − T` = 0.917. The whole range is below it, so the rule is attempted.

Not predicted: any change in the key compare (`bcmp`). A weaker hash could lengthen probe chains;
that would show up as a ratio above the range.

## Public API

`IndexData::key_lookup` is a `pub` field of a `pub` struct with all-`pub` fields, so changing its
hasher or adding a field to `IndexData` is a public-API change. **Route chosen (DECISIONS R24):** a
private second map with a fast hasher inside `MaterializedIndex` (`index_reader/lazy.rs`,
`pub(super)`), built from the same entries and read by `lookup_partition`. `key_lookup` stays as it
is. Cost: a second map per materialized index (about one `Arc` pointer pair, one `usize` and one
control byte per partition, plus load-factor slack; key bytes are shared through the `Arc`). The
cheaper public route is in `docs/rules/proposals/default-siphash-on-hot-map.md`.

## Outcome

(appended after the confirmation run)

### Outcome, 2026-11-23

Branch `r2/default-siphash-on-hot-map` at `8d2137eb` (one attempt). Fix: a private `FxHashMap`
beside `key_lookup` in `MaterializedIndex`, read by `lookup_partition`.

| | value | record |
|---|---|---|
| screen (attempt 1) | none: `NOISE_GATE_FAILED`; not rescreened, no attempt 2 | R2-000029 |
| review | internal only, no format change, no weakened tests; three minor notes | — |
| tests | no failure outside the base's list (16 failing, all pre-existing) | R2-000030 |
| **confirm, targets** | **WON**: ratio **0.4419**, 95% interval 0.4371 to 0.4458; b 0.0416, T 0.0832; 20 sessions, 0 rejected | R2-000031 |
| confirm, whole workload | WON, same numbers (the target set is the whole workload) | R2-000031 |
| cross-check read | no REGRESSION; workload ratio 0.9895 (0.9830 to 0.9958), T 0.05; per-case points 0.958 to 1.005; 10 sessions | R2-000032 |
| cross-check decode | no REGRESSION; workload ratio 0.9905 (0.9839 to 0.9960), T 0.05; per-case points 0.952 to 1.015; 10 sessions | R2-000033 |
| `perf-gate` 10% column | no case flagged in any of the three runs | R2-000031..33 |

Per-case confirm ratios: `borrow_trait_lookup` 0.340, `position_0` 0.350, `hot_keys` 0.473,
`sequential_access` 0.482, `random_access` 0.519, `1000_lookups` 0.528.

- **Prediction held: no.** The point estimate 0.4419 is below the predicted range (0.46 to 0.72):
  the change removed more than predicted. The prediction assumed the key compare would not change.
- **Paid off: yes.** WON with no REGRESSION.
- The read and decode cross-check intervals sit just below 1 and inside T. The read null's own
  interval (R2-000016, 0.9855 to 0.9975) does too, so this is not evidence of an effect there.
- Not measured: the memory and build time of the second map (the bench does not exercise them).
