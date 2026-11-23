# Screening attempts (Phase 3, provisional)

Screening numbers are provisional and are not verdicts.

| rule | attempt | branch sha | ledger id | target point ratio | note |
|---|---|---|---|---|---|
| default-siphash-on-hot-map | 1 | 8d2137ebd3891c3fbfd1b6b4f55eea507bebbc95 | R2-000029 | none: NOISE_GATE_FAILED (4 sessions rejected for load 4.65-6.52, max 3 replaced; 2 of 5 completed) | provisional; not a verdict; no rescreen run (one screen per attempt); attempt 2 not started |
| copy-cached-chunk-into-window | 1 | c1d91fb255a11b794a6a43d83d9294ad0d9e8506 | R2-000034 | clustering_point_bti 1.0034, clustering_range_bti 0.9983 (geomean 1.0008; ledger verdict TIED) | provisional; not a verdict; worse than predicted upper bound 0.934 |
| copy-cached-chunk-into-window | 2 | — | — | not attempted | attempt 1 chosen (head c1d91fb2). Warm target queries stitch 20 chunks (point) / 15 chunks (range) of the 38-chunk pk=1 partition, so no single cached chunk covers the parse; avoiding the stitch would need a sparse window tied to parser internals and would change add_chunk_decompressed counts (out of scope) |
