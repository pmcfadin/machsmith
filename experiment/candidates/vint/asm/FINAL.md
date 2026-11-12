FINAL: attempt-01

The candidate avoids A.r3's stack buffer, width-specific copies and push/pop frame: it uses a direct movbe load, a variable shift and bzhi to produce the value in about a dozen instructions, with a fast path for one-byte values. The advantage mostly comes from better instructions for the same logic (no store-to-load forwarding through a stack buffer, no frame setup), plus a bit from the data statistics (single-byte fast path is laid out as the first, fall-through case).
