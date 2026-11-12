# Trace capture

Recorded inputs for the `vint` and `bti_sparse_find` kernels come from CQLite's own
benchmark cases; `index_framing` inputs are the `*-big-Index.db` files in the datasets.

CQLite commit `2e5b7a4cf6abe6f51b7677ecb7199e2cab087c0e`, clone at `work/cqlite`.
`cqlite-trace.patch` is applied to the clone only (new module `cqlite-core/src/msx_trace.rs`,
hooks in `parser/vint.rs::decode_unsigned` and `storage/sstable/bti/node.rs::BtiNode::find_child`).
Tracing is off unless `MSX_TRACE_DIR` is set.

## Commands (all run by `capture.sh`, sequentially)

```
git -C work/cqlite apply experiment/trace-capture/cqlite-trace.patch
cd work/cqlite && CARGO_TARGET_DIR=$MSX_WORK/target/cqlite-trace cargo bench -p cqlite-core \
    --profile perfsym --features cli-helpers,bench-internals \
    --bench read --bench partition_lookup --bench m1_performance --no-run
<binary> --bench --list                      # per bench: read, partition_lookup, m1_performance
MSX_TRACE_DIR=work/traces-raw/<bench>/<case-slug>/ \
CQLITE_DATASETS_ROOT=/home/dev/projects/cqlite/test-data/datasets \
    <binary> --bench --test '^<case id, regex-escaped>$'        # once per case
```

The binaries are `work/target/cqlite-trace/perfsym/deps/{read,partition_lookup,m1_performance}-<hash>`.
Each case id is written to `<case dir>/ORIGIN` and becomes the segment's `origin`.
A failing case is listed in `work/traces-raw/failed-cases.txt` and skipped (none failed).

## Building the segments

```
msx_cargo dev run --manifest-path experiment/Cargo.toml -p msx-kernels --bin msx-trace-manifest -- \
    --raw work/traces-raw --datasets /home/dev/projects/cqlite/test-data/datasets --out experiment/traces
```

Outcome of the capture on 12 November 2026: 23 cases (9 `read`, 6 `partition_lookup`,
8 `m1_performance`), 0 failed. `vint` bytes were recorded for all; `bti_sparse_find` for none,
because `BtiNode::find_child` is not on the read path (see `../kernels/SELECTION.md`).
