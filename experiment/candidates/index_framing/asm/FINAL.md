FINAL: attempt-01

The candidate is a from-scratch straight-line version: it drops the compiler's stack frame, five pushes/pops, the 8-byte scratch buffer and the dead panic paths, and handles the dominant single-byte VInt case with one load and a sign test. The advantage (about 5.5%, near the tie threshold) comes from better instructions for the same logic plus the real-data statistic that VInts are almost always one byte; the multi-byte path is a simple byte loop.
