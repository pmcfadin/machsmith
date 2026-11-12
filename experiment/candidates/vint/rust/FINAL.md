FINAL: attempt-01

The candidate checks for empty input and a one-byte encoding first, in a frame-less fast path, and moves multi-byte decoding into a cold function using overlapping big-endian loads that stay inside the input. The gain comes mostly from better instructions for the same logic (no prologue/epilogue on the common path, no memcpy dispatch), plus the statistics of the real data (81% one-byte), which the PGO build of A.r3 did not exploit as fully.
