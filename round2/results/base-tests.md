# Base commit test suite (Task 8)

CQLite commit `7f9abe3663254724439dbe71aff77bb93faa1c2d`, worktree `work/cqlite-r2-wt/7f9abe366325-test`. Record of reference: **R2-000004**.

## Mode: fallback (deviation)

CQLite's agent gate cannot certify a clone: its preflight refuses with `component-set: FAIL-CLOSED (#3544) — baseline NOT measured (remote-not-canonical …)` because the clone's `origin` is a local path, not `github.com/pmcfadin/cqlite` (R2-000001). Per spec 7.4 the fallback runs instead (decisions R10–R12 in `round2/DECISIONS.md`):

| step | command | passed | failed | exit |
|---|---|---|---|---|
| workspace | `cargo test --workspace --no-fail-fast --exclude cqlite-py --exclude cqlite-node` | 9,622 | 17 | 101 |
| core_cli_helpers | `cargo test --no-fail-fast -p cqlite-core --features cli-helpers` (the gate's core-tests component) | 6,428 | 12 | 101 |
| clippy | `RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets --all-features` | — | — | 0 |

Tests built without debuginfo (`CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_PROFILE_TEST_DEBUG=0`), target dir on the cache volume. The Python and Node binding crates are not tested (they cannot link as Rust test binaries); round 2 changes stay inside `cqlite-core`.

Earlier attempts and why they produced no usable baseline: R2-000001 (gate refused, before the fallback rule covered this cause), R2-000002 (run-id race in `test.sh`), R2-000003 (`cargo test --workspace` failed to link `cqlite-py`, 0 tests), try 4 (no record: the test build filled the shared projects volume; incident in the SDD ledger, fixed by R12).

## Pre-existing failures (17 distinct tests)

These fail on the unchanged base and are therefore "pre-existing" for every branch. A branch is blocked only by failures **not** in this list.

- `digest_crc32_byte_for_byte_parity`
- `gc_before_boundary_matches_the_sstabledump_golden`
- `partition_tombstone_generation_still_yields_one_row`
- `point_key_yields_one_row_per_generation_no_reconciliation`
- `resurrection_gc0_matches_the_sstabledump_golden`
- `resurrection_gc_positive_matches_the_sstabledump_golden`
- `skipped_partition_delete_matches_the_sstabledump_golden`
- `sstable_parity_corruption_verify_matches_cassandra_per_fixture`
- `statistics_db_strict_core_metadata_parity`
- `statistics_max_ldt_and_tombstone_histogram_parity`
- `storage::write_engine::merge::async_bridge::tests::concurrent_block_on_from_many_threads_is_safe_and_parallel` (workspace step only; a timing/parallelism test on a loaded host — likely flaky)
- `test_corrupted_file_handling` (workspace only)
- `test_csv_output_snapshot` (workspace only)
- `test_json_output_deterministic` (workspace only)
- `test_table_output_snapshot` (workspace only)
- `tombstone_histogram_matches_the_sstabledump_golden`
- `wide_range_tombstone_matches_the_sstabledump_golden`

The suite is usable as a referee: 16,050 passing tests, 29 failing results across 17 names, all on the base. Why they fail on the base was not investigated (out of scope; CQLite's gate applies a CI skip-list that this fallback does not).
