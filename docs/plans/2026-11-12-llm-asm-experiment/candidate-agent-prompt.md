# Candidate agent prompt (template)

The orchestrator fills in every `{{placeholder}}` and sends the text below the line, and nothing else, to a fresh agent. Placeholders:

| Placeholder | Value |
|---|---|
| `{{KERNEL}}` | kernel name, for example `vint` |
| `{{ARM}}` | `asm` or `rust` |
| `{{EXT}}` | `s` for `asm`, `rs` for `rust` |
| `{{LANGUAGE}}` | "x86-64 assembly" or "Rust" |
| `{{AGENT_ID}}` | `{{KERNEL}}-{{ARM}}` |
| `{{MODEL}}` | the model id the agent runs on |
| `{{ARM_RULES}}` | the "Assembly arm" or "Rust arm" block from the end of this file |

---

You are taking part in an experiment that asks whether a language model can write code that beats an optimizing compiler.

## Your job

Write the fastest correct implementation you can of one small function, the `{{KERNEL}}` kernel, in {{LANGUAGE}}, for one specific CPU. You are competing against the Rust compiler's output for the same function at three optimization levels. You get a limited number of measured attempts. The result will be published whether you win, tie or lose, so an honest tie is better than a broken win.

## What you have

Everything you need is in `/home/dev/projects/machsmith/experiment/candidates/{{KERNEL}}/BRIEF/`:

- `CARD.md`: what the function does, its exact C signature, its contract, what you may and may not assume about its inputs, how it is called, statistics of the real inputs, the original Rust source, and the CPU's features.
- `A.r1.s`, `A.r2.s`, `A.r3.s`: the compiler's machine code for this function at each level. `A.r1` is a generic x86-64 release build. `A.r2` adds tuning for this CPU. `A.r3` adds profile-guided optimization and is the one to beat.
- `A.r3.assemblable.s` (may be absent): `A.r3`'s code as a file that passes the rules below. A legal starting point for the assembly arm.
- `baseline.json`: measured cycles, instructions and branch misses per operation for each compiler level on the real inputs.
- `RULES.md`: the rules below, for reference.

Read `CARD.md` completely before writing anything.

## Rules

1. **Work only in `/home/dev/projects/machsmith/experiment/candidates/{{KERNEL}}/{{ARM}}/`.** Create and edit files only there. Do not edit, create or delete anything else in the repository. The attempt command checks, and refuses to run if anything else has changed.
2. **Do not read** the other arm's directory (`candidates/{{KERNEL}}/` other than `BRIEF/` and your own `{{ARM}}/`), any other kernel's `candidates/` directory, anything under `experiment/traces/*/holdout/`, `experiment/selftest/`, or `experiment/results/`. You may read `experiment/traces/{{KERNEL}}/search/` and its `MANIFEST.json` to understand the inputs.
3. **Do not build or run your candidate yourself**, and do not run benchmarks or other heavy processes: the machine is shared and stray load corrupts everyone's measurements. The attempt command is the only way to build, test and time a candidate. Reading files and thinking are free.
4. **The contract is exact.** Your function must return the same value and write the same outputs as the original for every possible input, including malformed and truncated ones, and must not write to the output on an error return.
5. **Stay inside the input.** You may read only the bytes from `ptr` up to but not including `ptr + len`, and write only the output the signature gives you. The tests place the input directly against unreadable memory on both sides, at many lengths. A load that reaches one byte outside crashes and fails the attempt, even if its result is never used.
6. **No memorizing inputs.** Read-only data in your candidate is capped (see the arm rules). Solve the general problem.
7. **Follow the System V x86-64 calling convention.** Preserve `rbx`, `rbp`, `r12`-`r15` and `rsp`; return with the direction flag clear and `MXCSR` unchanged; if you use AVX or AVX-512 registers, execute `vzeroupper` before returning.
8. Any instruction this CPU supports is allowed, except those the arm rules forbid.

{{ARM_RULES}}

## The attempt command

Save a candidate as `attempt-NN.{{EXT}}` in your directory, where `NN` is `01` for your first submission and increases by one with every submission, whatever its outcome. Then run exactly:

```bash
cd /home/dev/projects/machsmith/experiment && . env.sh && \
MSX_AGENT_ID={{AGENT_ID}} MSX_MODEL={{MODEL}} \
"$MSX_WORK/target/tool/release/msx" attempt --kernel {{KERNEL}} --arm {{ARM}} \
  --file candidates/{{KERNEL}}/{{ARM}}/attempt-NN.{{EXT}}
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

## Budget

- **10 counted attempts.** Every submission that reaches the correctness test counts, whether it passes or fails.
- **30 submissions in total**, including the free lint and build failures.

Ten is few. Reason carefully about correctness before you submit, especially at the edges: empty input, every truncation point, the longest encodings. Use early attempts to learn what the measurements reward; do not spend them on variations you cannot tell apart.

## Final exam

When you are done, your chosen candidate is tested once more, without you: on twenty million fresh inputs derived from your file's own hash, on real inputs you have never seen, and with a much longer measurement. A candidate that fails there is disqualified with no second choice. Prefer a candidate you are sure is correct over one that is slightly faster and less certain. Differences from `A.r3` smaller than about 5% will be reported as a tie.

## What to leave behind

In your directory:

1. `NOTES.md`: for each submission, one short paragraph: what you tried, why you expected it to help, and what happened. Be candid about what did not work. This is used in the published write-up.
2. `FINAL.md`: the first line must be exactly `FINAL: attempt-NN`, naming your chosen passing attempt, or `FINAL: none` if no attempt passed. Below it, in a few sentences: what your final candidate does differently from `A.r3`, and whether its advantage (if any) comes from better instructions for the same logic, from something true about this function's inputs that the compiler could not know, or from the statistics of the real data.

Then stop and report, in a few lines: your final choice, its provisional ratio against `A.r3`, and how many attempts you used.

---

## Arm rules blocks

### Assembly arm

**Assembly arm rules**

- One file in GNU assembler syntax (AT&T, or Intel with `.intel_syntax noprefix`), assembled by `gcc -c -x assembler` with no C preprocessor.
- It must define the global function `msx_{{KERNEL}}`, with `.type msx_{{KERNEL}}, @function`, a `.size` directive, and `.p2align 6` immediately before its label (the entry must be 64-byte aligned).
- End the file with `.section .note.GNU-stack,"",@progbits`.
- Allowed sections: `.text`, `.rodata`. At most 4,096 bytes of `.rodata`. At most 8,192 bytes of code. No writable data, no initializers.
- The function must be a leaf: no `call` at all, no jumps out of the file, no external symbols, no relocations that need the dynamic loader (address your constants `rip`-relative).
- Forbidden: indirect jumps and calls (so no jump tables through a register or memory operand), `syscall`, `sysenter`, `int`, `rdtsc`, `rdtscp`, `rdpmc`, `rdrand`, `rdseed`, `cpuid`, `xgetbv`, `.include`, `.incbin`.
- You may use the red zone (the 128 bytes below `rsp`).

### Rust arm

**Rust arm rules**

- One Rust file. It is compiled as the module `candidate` of a `#![no_std]` crate with `opt-level=3`, fat LTO, `panic=abort`, `-C target-cpu=znver5` and profile-guided optimization trained on the real inputs. You do not control the flags.
- It must define exactly the function in `CARD.md`: `#[no_mangle] pub unsafe extern "C" fn msx_{{KERNEL}}(…)` with that signature. Helper functions, constants and `#[repr(C)]` types in the same file are fine.
- Only `core` is available. Calls the compiler itself emits to `memcpy`, `memmove`, `memset`, `memcmp` or `bcmp` are allowed and are resolved to the C library, as they are for the compiler's own builds. `unsafe` code and `core::arch::x86_64` intrinsics are allowed; you may assume every feature listed in `CARD.md` is present, and use `#[target_feature(enable = "…")]` on helpers if needed.
- Forbidden: `asm!`, `global_asm!`, naked functions, `std`, `alloc`, `extern` declarations other than your one function, `include!`, `include_bytes!`, `include_str!`, `static mut`, crate-level attributes (`#![…]`), `#[link…]`.
- A panic crashes the process and fails the attempt. Make sure no input can reach one.
- At most 16,384 bytes of read-only data and 8,192 bytes of code in `msx_{{KERNEL}}`.
- Your goal is a rewrite that makes the compiler produce faster code. You are not being asked to hand-write the machine code.
