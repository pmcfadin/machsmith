# A.reasm: the compiler's assembly through the candidate path

A.reasm is `A.r3`'s assembly for `msx_<kernel>`, cut out and built with the assembly-candidate
path. It is a second null (identical code through a different build path). No `<kernel>.s` exists
in this directory, for these reasons:

- `vint`: the compiler's output calls `memcpy` through the GOT, an external symbol the candidate
  rules forbid (verified 12 November 2026). Its null is `A.r3copy` alone and its Rust-path control
  is `vint/control_copy.rs`.
- `index_framing`: producing it was attempted (A.r3 flags plus `--emit=asm`, into
  `work/target/A-r3-asm`). `msx_index_framing` references symbols outside itself:
  `memcpy`, `core::panicking::panic_fmt` and `core::slice::index::slice_index_fail`, all through
  the GOT. A.reasm is not producible and the code was not edited to make it so. The Rust-path
  stand-in is absent for this kernel; its null is `A.r3copy` alone.
- `bti_sparse_find`: a dropped kernel with no recorded traces; there is nothing to measure, so no
  files exist for it here or under `selftest/bti_sparse_find/`.

`msx null` and `msx final` already treat a missing `work/build/A/reasm/<kernel>.so` as "no
A.reasm". If a `<kernel>.s` is ever added here, list it in `EXPECT.json` as `"<kernel>.s":
"PASS/TIED"`; `msx selftest` builds it into `work/build/A/reasm/<kernel>.so`.
