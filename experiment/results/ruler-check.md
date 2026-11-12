# Ruler check (Task 11)

Kernels: vint, index_framing (bti_sparse_find dropped in Task 4). No A.reasm exists, so each null is A.r3copy vs A.r3 only.
Tool: frozen msx, sha256 9007b7ac...e9f14bd. Protocol: 30 sessions x 200 blocks. Threshold T on the band: 0.05.

| kernel | try | ledger id | null interval (r3copy/r3) | point | band | T | result |
|---|---|---|---|---|---|---|---|
| vint | 1 | L000005 | [0.99997, 1.00024] | 1.00011 | 0.00024 | 0.05 | pass |
| index_framing | 1 | L000006 | [0.99926, 1.00002] | 0.99969 | 0.00074 | 0.05 | pass |

Check (phase final, seed from card): r2, r3, r3copy all PASS for both kernels (40698122 cases for vint, 40762802 for index_framing); r3copy and r3 are byte-identical (same sha256).

Meaning: the measured bands are 0.02% and 0.07%, far under the 5% gate, so this machine can resolve ratio differences well below 1% for these two kernels. Effects of a few percent are visible; a candidate within about +-0.1% of rustc is indistinguishable from a tie. Caveat: A.r3copy and A.r3 are the same bytes, so the null shows noise at one code placement only, and it cannot reveal layout or alignment sensitivity between different binaries.
