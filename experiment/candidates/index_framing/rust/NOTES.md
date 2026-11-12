# Notes

**attempt-01** (PASS, ratio vs A.r3 0.984 [0.983, 0.985], 25.6 vs 26.0 cycles/op). Rewrote the nom/slice code as plain pointer arithmetic with position checks only: one `pos >= len` check after the key, one after the first VInt width, and a fast path for a one-byte second VInt (the typical promoted length 0). Multi-byte second VInts go to a cold out-of-line byte loop (no 8-byte scratch buffer, no memcpy calls, no panic paths). Instruction count fell from 84.6 to 49.3 per op, but cycles barely moved: the loop is bound by the dependent chain length -> key bytes -> first VInt byte -> second VInt byte, so fewer instructions bought only ~1.6%, within the tie band.

**attempt-02** (PASS, ratio 1.058, slower). Same, but branch on `b1 < 0x80` to skip the lzcnt for the first VInt, hoping speculation shortens the chain. It got slower (27.5 cycles); likely the extra branch/layout cost outweighed any gain, since the first VInt is often multi-byte in real data (data offsets are large).
