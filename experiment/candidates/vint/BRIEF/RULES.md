## Rules

1. **Work only in `/home/dev/projects/machsmith/experiment/candidates/vint/<arm>/`.** Create and edit files only there. Do not edit, create or delete anything else in the repository. The attempt command checks, and refuses to run if anything else has changed.
2. **Do not read** the other arm's directory (`candidates/vint/` other than `BRIEF/` and your own `<arm>/`), any other kernel's `candidates/` directory, anything under `experiment/traces/*/holdout/`, `experiment/selftest/`, or `experiment/results/`. You may read `experiment/traces/vint/search/` and its `MANIFEST.json` to understand the inputs.
3. **Do not build or run your candidate yourself**, and do not run benchmarks or other heavy processes: the machine is shared and stray load corrupts everyone's measurements. The attempt command is the only way to build, test and time a candidate. Reading files and thinking are free.
4. **The contract is exact.** Your function must return the same value and write the same outputs as the original for every possible input, including malformed and truncated ones, and must not write to the output on an error return.
5. **Stay inside the input.** You may read only the bytes from `ptr` up to but not including `ptr + len`, and write only the output the signature gives you. The tests place the input directly against unreadable memory on both sides, at many lengths. A load that reaches one byte outside crashes and fails the attempt, even if its result is never used.
6. **No memorizing inputs.** Read-only data in your candidate is capped (see the arm rules). Solve the general problem.
7. **Follow the System V x86-64 calling convention.** Preserve `rbx`, `rbp`, `r12`-`r15` and `rsp`; return with the direction flag clear and `MXCSR` unchanged; if you use AVX or AVX-512 registers, execute `vzeroupper` before returning.
8. Any instruction this CPU supports is allowed, except those the arm rules forbid.

## Arm rules

Only the block for your arm applies.

### Assembly arm

**Assembly arm rules**

- One file in GNU assembler syntax (AT&T, or Intel with `.intel_syntax noprefix`), assembled by `gcc -c -x assembler` with no C preprocessor.
- It must define the global function `msx_vint`, with `.type msx_vint, @function`, a `.size` directive, and `.p2align 6` immediately before its label (the entry must be 64-byte aligned).
- End the file with `.section .note.GNU-stack,"",@progbits`.
- Allowed sections: `.text`, `.rodata`. At most 4,096 bytes of `.rodata`. At most 8,192 bytes of code. No writable data, no initializers.
- The function must be a leaf: no `call` at all, no jumps out of the file, no external symbols, no relocations that need the dynamic loader (address your constants `rip`-relative).
- Forbidden: indirect jumps and calls (so no jump tables through a register or memory operand), `syscall`, `sysenter`, `int`, `rdtsc`, `rdtscp`, `rdpmc`, `rdrand`, `rdseed`, `cpuid`, `xgetbv`, `.include`, `.incbin`.
- You may use the red zone (the 128 bytes below `rsp`).

### Rust arm

**Rust arm rules**

- One Rust file. It is compiled as the module `candidate` of a `#![no_std]` crate with `opt-level=3`, fat LTO, `panic=abort`, `-C target-cpu=znver5` and profile-guided optimization trained on the real inputs. You do not control the flags.
- It must define exactly the function in `CARD.md`: `#[no_mangle] pub unsafe extern "C" fn msx_vint(…)` with that signature. Helper functions, constants and `#[repr(C)]` types in the same file are fine.
- Only `core` is available. Calls the compiler itself emits to `memcpy`, `memmove`, `memset`, `memcmp` or `bcmp` are allowed and are resolved to the C library, as they are for the compiler's own builds. `unsafe` code and `core::arch::x86_64` intrinsics are allowed; you may assume every feature listed in `CARD.md` is present, and use `#[target_feature(enable = "…")]` on helpers if needed.
- Forbidden: `asm!`, `global_asm!`, naked functions, `std`, `alloc`, `extern` declarations other than your one function, `include!`, `include_bytes!`, `include_str!`, `static mut`, crate-level attributes (`#![…]`), `#[link…]`.
- A panic crashes the process and fails the attempt. Make sure no input can reach one.
- At most 16,384 bytes of read-only data and 8,192 bytes of code in `msx_vint`.
- Your goal is a rewrite that makes the compiler produce faster code. You are not being asked to hand-write the machine code.

## The attempt command

Here `<arm>` is `asm` or `rust`, and `<ext>` is `s` for `asm` and `rs` for `rust`. Save a candidate as `attempt-NN.<ext>` in your directory, where `NN` is `01` for your first submission and increases by one with every submission, whatever its outcome. Then run exactly:

```bash
cd /home/dev/projects/machsmith/experiment && . env.sh && \
MSX_AGENT_ID=vint-<arm> MSX_MODEL=claude-sonnet-5-5 \
"$MSX_WORK/target/tool/release/msx" attempt --kernel vint --arm asm|rust \
  --file candidates/vint/<arm>/attempt-NN.<ext>
```

It can take several minutes and may wait for another measurement to finish first. Let it run; do not start a second one. It prints one JSON object:

- `verdict`:
  - `LINT_FAILED` or `BUILD_FAILED`: a rule or the assembler/compiler rejected the file. See `lint_errors` or `build_stderr`. **Free**: does not use up a counted attempt.
  - `INCORRECT`: wrong output on some input. `first_failure` shows one failing input (hex), what was expected and what you returned.
  - `OOB_OR_CRASH`: your code crashed, almost always by reading or writing outside the input. `first_failure` shows the input.
  - `ABI_VIOLATION`: you broke the calling convention; `first_failure.abi` says how.
  - `TIMEOUT`: your code did not finish.
  - `PASS`: correct on millions of generated test cases plus every recorded real input in the search set. `timing` is filled in.
  - `BUDGET_EXHAUSTED`: you have no attempts left.
- `timing.ratio_vs`: your cycles per operation divided by each compiler level's, with a 95% interval. Below 1.0 means you are faster. These search-phase numbers are **provisional**: they are noisier than the final measurement and are not the published result.
- `timing.cycles_per_op`, `instructions_per_op`, `branch_misses_per_op`: yours (`cand`) and each compiler level's.
- `counted_attempts_left`, `submissions_left`.
