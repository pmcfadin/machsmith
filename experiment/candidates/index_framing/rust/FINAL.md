FINAL: attempt-01

Attempt-01 does the same logic as A.r3 with fewer instructions (about 49 vs 85 per op): no scratch buffer or panic paths, and a fast path for a one-byte second VInt. The gain comes from better instructions for the same logic, plus the real-data fact that the promoted length is almost always a single byte. The advantage is about 1.6%, so expect a tie, because the dependent load chain dominates the cycle count.
