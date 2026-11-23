# Screening attempts (Phase 3, provisional)

Screening numbers are provisional and are not verdicts.

| rule | attempt | branch sha | ledger id | target point ratio | note |
|---|---|---|---|---|---|
| default-siphash-on-hot-map | 1 | 8d2137ebd3891c3fbfd1b6b4f55eea507bebbc95 | R2-000029 | none: NOISE_GATE_FAILED (4 sessions rejected for load 4.65-6.52, max 3 replaced; 2 of 5 completed) | provisional; not a verdict; no rescreen run (one screen per attempt); attempt 2 not started |
| copy-cached-chunk-into-window | 1 | c1d91fb255a11b794a6a43d83d9294ad0d9e8506 | R2-000034 | clustering_point_bti 1.0034, clustering_range_bti 0.9983 (geomean 1.0008; ledger verdict TIED) | provisional; not a verdict; worse than predicted upper bound 0.934 |
| copy-cached-chunk-into-window | 2 | — | — | not attempted | attempt 1 chosen (head c1d91fb2). Warm target queries stitch 20 chunks (point) / 15 chunks (range) of the 38-chunk pk=1 partition, so no single cached chunk covers the parse; avoiding the stitch would need a sparse window tied to parser internals and would change add_chunk_decompressed counts (out of scope) |
| type-name-match-per-clustering-value | 1 | c498e714cab3e58fa4ec73541289b48060916c65 | R2-000037 | clustering_point_bti 0.9872, clustering_range_bti 0.9893 (geomean 0.9883; ledger verdict TIED) | provisional; not a verdict; worse than predicted upper bound 0.937; per-session point ratios 0.877-1.089 |
| type-name-match-per-clustering-value | 2 | — | — | not attempted | attempt 1 chosen (head c498e714). The bench's ck is `int`: after attempt 1 the per-value work is five length-only rejects plus one 3-byte compare, with no allocation. A per-table cache would need state on a schema/parser type for no measurable remaining cost |
