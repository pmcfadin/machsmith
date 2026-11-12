# Phase 2: the harness (Tasks 5-10)

Read `README.md` in this directory first. Types, paths, CLI commands, verdict strings and constants named here are defined there.

All cargo commands run as `. experiment/env.sh && msx_cargo <target-name> … --manifest-path experiment/Cargo.toml`. Harness tests use target name `dev`.

Internal subcommands of `msx` that start with two underscores (`__check-child`, `__session`, `__train`) are implementation details run only by `msx` itself in child processes.

---

### Task 5: Loader, guard buffers, ABI trampoline, kernel specs

**Files:**
- Create: `experiment/harness/src/loader.rs`, `guard.rs`, `tramp.rs`, `spec.rs`, `spec_vint.rs`, `spec_index_framing.rs`, `spec_bti_sparse_find.rs`
- Create: `experiment/harness/tests/fixtures/{ok_sum.s,clobber_rbx.s,set_df.s,dirty_ymm.s}`, `experiment/harness/tests/support/mod.rs`
- Test: `experiment/harness/tests/tramp.rs`, `tests/guard.rs`, `tests/spec_mirror.rs`

**Interfaces:**
- Consumes: `msx-kernels` as a dev-dependency (its `extern "C"` functions cast to `RawFn`), traces from Task 4.
- Produces:

```rust
// loader.rs
pub struct Lib { /* never dlclosed */ }
impl Lib { pub fn open(path: &Path) -> Result<Lib, String>; pub fn sym(&self, name: &str) -> Option<RawFn>; }

// guard.rs
pub struct GuardBuf { /* mmap: [PROT_NONE page][rw pages][PROT_NONE page] */ }
impl GuardBuf {
    pub fn new(max_len: usize) -> GuardBuf;
    pub fn place_end(&mut self, bytes: &[u8]) -> *const u8;    // last byte sits immediately before the trailing guard page
    pub fn place_start(&mut self, bytes: &[u8]) -> *const u8;  // first byte sits immediately after the leading guard page
}
pub struct OutSlot { /* exactly `words * 8` writable bytes ending at a guard page */ }
impl OutSlot {
    pub fn new(words: usize) -> OutSlot;
    pub fn arm(&mut self) -> *mut u64;                // fills with 0xDEAD_BEEF_DEAD_BEEF, returns pointer
    pub fn take(&self) -> ([u64; 3], bool);           // values (unused words zero), and whether anything changed
}

// tramp.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum AbiViolation { CalleeSaved, StackPointer, DirectionFlag, Mxcsr, UpperVectorState }
/// Call `f(a, b, c, d)` under the System V ABI with hostile register state, and audit the state on return.
pub unsafe fn call4(f: RawFn, a: u64, b: u64, c: u64, d: u64) -> Result<u64, AbiViolation>;

// test support (tests/support/mod.rs)
pub fn assemble_fixture(name: &str) -> PathBuf;   // gcc -c + gcc -shared -nostdlib -nostartfiles into a temp dir
```

plus `KernelSpec`, `OutRec`, `kernel()`, `all_kernels()` exactly as in the README.

**What `call4` must do** (this is the part the signature does not determine):

Before the call: save Rust's callee-saved registers; load `rbx`, `rbp`, `r12`-`r15` with six distinct canary values; load `rax`, `r10`, `r11` and every argument register not used by the kernel with garbage; execute `vzeroupper`; record `rsp` and `MXCSR`; ensure `rsp` is 16-byte aligned at the `call`. For an argument the kernel declares as 32-bit (`byte: u32`), the spec passes garbage in the upper 32 bits of `c`/`d`: the ABI leaves them undefined.

After the call, in this order: read `RFLAGS` and then execute `cld` unconditionally (Rust code assumes the direction flag is clear); compare the six canaries → `CalleeSaved`; compare `rsp` → `StackPointer`; direction flag was set → `DirectionFlag`; `MXCSR` changed → `Mxcsr`; if `CPUID.(EAX=0DH,ECX=1):EAX` bit 2 is set, execute `xgetbv` with `ecx = 1` and report `UpperVectorState` when bit 2 (`YMM_Hi128`) or bit 6 (`ZMM_Hi256`) is set, then execute `vzeroupper`. Restore Rust's registers. Write it with `core::arch::global_asm!` or `asm!`; both are stable.

**What each spec must do:**

- `run_case`: place nothing itself; it receives the already-placed `input` pointer. It arms an `OutSlot` sized for the kernel's out-struct, calls through `call4`, and returns `OutRec { ret, out, out_written }`.
- `run_chain`: the same loop as `msx_<kernel>_chain` in Task 3 (same stop rule, same checksum fold, same advance), calling the kernel through a plain function-pointer call, not through `call4`. This is the measured path; keep it minimal.
- `chunks`: walks with the reference and cuts at operation boundaries so each chunk has at least `min_ops` operations; a tail shorter than `min_ops` is dropped. A segment with fewer than `min_ops` operations yields no chunks (small-trace mode in Task 8 uses whole segments instead).
- `edge_cases` for `vint`: every lead byte 0..=255 × every length 0..=12 × ten tails (all `0x00`, all `0xFF`, eight from a ChaCha stream seeded `0xED6E`): 33,280 cases.
- `edge_cases` for `index_framing`: for key length in {0, 1, 2, 3, 7, 8, 9, 15, 16, 17, 31, 32, 33, 63, 64, 65, 255, 256, 1000, 65535}, offset varint width 1..=9 and promoted varint width 1..=9 (smallest and largest value of each width), build the valid entry, then emit it and its truncations: every prefix if the entry is at most 300 bytes, else prefixes of length 0..=4, key end ±2, and each of the last 20 lengths.
- `edge_cases` for `bti_sparse_find`: `n` in 0..=20 and {31, 32, 33, 63, 64, 65, 127, 128, 129, 255, 256}; for each, a sorted list of `n` distinct random bytes (for `n = 256`, all bytes), queried with **every** byte value 0..=255, the upper 24 bits of the `byte` argument filled with random garbage, struct padding filled with random bytes. The input pointer covers exactly `n × 16` bytes; `aux` carries the `byte` argument.
- `random_cases`: half uniformly random bytes of length 0..=64; half structurally valid inputs built from random fields, then truncated at a random point with probability 1/2.

- [ ] **Step 1: Write the failing tests**

`tests/tramp.rs` (fixtures are tiny hand-written `.s` files taking four integer arguments):
- `ok_sum.s` returns `a + b + c + d` and touches only caller-saved registers → `call4(f, 1, 2, 3, 4) == Ok(10)`.
- `clobber_rbx.s` zeroes `rbx` and returns → `Err(AbiViolation::CalleeSaved)`.
- `set_df.s` executes `std` and returns → `Err(AbiViolation::DirectionFlag)`, and a following Rust `copy_from_slice` in the test still copies forward correctly.
- `dirty_ymm.s` executes `vpcmpeqd %ymm0, %ymm0, %ymm0` and returns → `Err(AbiViolation::UpperVectorState)` (skipped with a notice if the CPUID bit is absent).

`tests/guard.rs`:
- `place_end` of a 5-byte slice: reading bytes 0..5 through the pointer equals the slice; in a forked child, reading byte 5 dies with `SIGSEGV`.
- `place_start`: in a forked child, reading the byte before the pointer dies with `SIGSEGV`.
- `OutSlot::new(1)`: `take()` after `arm()` reports unchanged; after writing one word reports changed with that value; in a forked child, writing a second word dies with `SIGSEGV`.

`tests/spec_mirror.rs`, for every kernel and every **search** segment:
- `run_chain(A, whole segment)` returns exactly what `msx_<kernel>_chain` returns (operations and checksum), where `A` is the `msx-kernels` function pointer;
- the sum of operations over `chunks(A, seg, 1000)` plus the dropped tail's operations equals the manifest's `ops`;
- `trace_cases` yields exactly `ops` cases.
- `edge_cases` yields the same sequence on two calls, and for `vint` exactly 33,280 cases.

Run: `msx_cargo dev test -p msx-harness` — expected: compile failures.

- [ ] **Step 2: Implement `loader.rs`, `guard.rs`, `tramp.rs`**
- [ ] **Step 3: Implement `spec.rs` and the three kernel specs**
- [ ] **Step 4: Run the tests**

Run: `msx_cargo dev test -p msx-harness` — expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add experiment/harness && git commit -m "feat: loader, guard buffers, ABI trampoline and kernel specs"
```

---

### Task 6: Rung builds with PGO, candidate builds, entry alignment

**Files:**
- Create: `experiment/harness/src/build.rs`
- Create: `experiment/candidate-wrapper/lib.rs`
- Create: `experiment/selftest/vint/control_copy.rs`, `experiment/harness/tests/fixtures/bad_syntax.s`
- Modify: `experiment/harness/src/bin/msx.rs` (`rungs build`, `__train`)
- Test: `experiment/harness/tests/build.rs`

**Interfaces:**
- Consumes: `run_chain`, `kernel()`, `all_kernels()` (Task 5); search segments (Task 4).
- Produces:

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Rung { R1, R2, R3, R3Copy }
pub struct BuildError { pub stage: &'static str, pub stderr: String }

pub fn build_a(rung: Rung) -> Result<PathBuf, BuildError>;       // work/build/A/<r1|r2|r3|r3copy>/libmsx_kernels.so
pub fn a_path(rung: Rung) -> PathBuf;                             // same path without building
pub fn build_asm_candidate(kernel: &str, src: &Path, out_dir: &Path) -> Result<PathBuf, BuildError>;   // <out_dir>/cand.so
pub fn build_rust_candidate(kernel: &str, src: &Path, recipe: Rung, out_dir: &Path) -> Result<PathBuf, BuildError>;
        // recipe R3 -> <out_dir>/cand.so ; recipe R2 -> <out_dir>/cand-r2.so
pub fn verify_profile(show_output: &str, symbols: &[&str]) -> Result<(), String>;   // parses `llvm-profdata show --all-functions`
pub fn entry_alignment(so: &Path, symbol: &str) -> Result<u64, String>;             // symbol address % 64, via llvm-nm
```

**Recipes** (the decisions; flags are passed through `RUSTFLAGS` for cargo builds):

| Build | Flags beyond `[profile.release]` | Cargo target dir |
|---|---|---|
| `A.r1` | `-C llvm-args=-align-all-functions=6` | `work/target/A-r1` |
| `A.r2` | r1 + `-C target-cpu=znver5` | `work/target/A-r2` |
| `A.r3` instrumented | r2 + `-C profile-generate=work/pgo/A/raw` | `work/target/A-r3-gen` |
| `A.r3` | r2 + `-C profile-use=work/pgo/A/merged.profdata -C llvm-args=-pgo-warn-missing-function` | `work/target/A-r3` |
| `A.r3copy` | identical to `A.r3`, same profile | `work/target/A-r3copy` |

After each cargo build, copy `libmsx_kernels.so` to its `work/build/A/<rung>/` path.

**PGO training** (`msx __train --so <instrumented.so> --kernels all`): a child process with `LLVM_PROFILE_FILE=work/pgo/A/raw/%m.profraw` that loads the instrumented library, runs every **search** segment of every kernel through `run_chain` three times, and returns from `main` normally so the profile runtime's exit hook writes the file. Do not call `process::exit` or `_exit` in this path. Then `llvm-profdata merge -o work/pgo/A/merged.profdata work/pgo/A/raw`.

**Profile must be real:** after merging, run `llvm-profdata show --all-functions --counts` and pass its output to `verify_profile` with every `msx_<kernel>` symbol. Per function the tool prints the name, `Hash:`, `Counters: N` and `Block counts: [a, b, …]`; there is no `Function count:` line. `verify_profile` requires each named function to be present with at least one nonzero block count. (Observed on this host: a function that training never called has all-zero block counts, so this check catches a training driver that exercises only the `_chain` symbols.) `build_a(R3)` fails unless the check passes. It also fails if the profile-use build's stderr contains `hash mismatch` or `no profile data available for function msx_`.

**Assembly candidate build:**

```bash
gcc -c -x assembler -o cand.o <src>            # no C preprocessor
gcc -shared -nostdlib -nostartfiles -Wl,-z,noexecstack -Wl,--build-id=none -o cand.so cand.o
```

**Rust candidate build.** The tool copies `experiment/candidate-wrapper/lib.rs` and the candidate file (as `candidate.rs`) into `<out_dir>/src/` and compiles there. `lib.rs` is:

```rust
#![cfg_attr(not(msx_train), no_std)]
mod candidate;
#[cfg(not(msx_train))]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    // A panicking candidate must crash, not hang: the checker reports it as OOB_OR_CRASH.
    unsafe { core::ptr::null_mut::<u8>().write_volatile(0) };
    loop {}
}
```

Common flags: `--edition 2021 --crate-name msx_cand -C opt-level=3 -C codegen-units=1 -C lto=fat -C panic=abort -C target-cpu=znver5 -C llvm-args=-align-all-functions=6`.

- Recipe R2: `rustc --crate-type staticlib <common> lib.rs -o cand.a`, then
  `gcc -shared -nostdlib -nostartfiles -Wl,--gc-sections -Wl,-u,msx_<kernel> -Wl,-z,noexecstack -Wl,--build-id=none -o cand-r2.so cand.a`.
- Recipe R3: first an instrumented training build, `rustc --crate-type cdylib --cfg msx_train <common> -C profile-generate=<out_dir>/pgo/raw lib.rs`; train it with `msx __train --so … --kernels <kernel>` and `LLVM_PROFILE_FILE=<out_dir>/pgo/raw/%m.profraw`; merge; `verify_profile` for `msx_<kernel>`; then the R2 command with `-C profile-use=<out_dir>/pgo/merged.profdata` added, output `cand.so`.

The crate name is the same in both builds so profile function names match. The instrumented training library is never linted or measured. The profile-use build prints `no profile data available for function …rust_begin_unwind`: that is the panic handler, which exists only in the `no_std` build; ignore that one warning. A candidate that copies a variable number of bytes links with `memcpy` left undefined; that is expected and the lint allows it (Task 7).

**Entry alignment:** `build_a`, `build_asm_candidate` and `build_rust_candidate` each call `entry_alignment` on every `msx_<kernel>` symbol they export. For `A` builds a nonzero result is a `BuildError`. If rustc rejects `-align-all-functions` outright, remove the flag from every recipe, make `entry_alignment` report-only, record each contestant's entry offset in the ledger, and add "entry alignment not normalized" to the limitations list in Task 14. Do not silently continue with mixed settings.

- [ ] **Step 1: Write the failing tests**

`tests/build.rs`:
- `r1_and_r2_build_and_export_aligned_symbols`: `build_a(R1)` and `build_a(R2)` succeed; each exports every `msx_<kernel>` and `msx_<kernel>_chain`; `entry_alignment` is 0 for every `msx_<kernel>`.
- `r3_profile_is_real`: `build_a(R3)` succeeds and `work/pgo/A/merged.profdata` exists; `verify_profile` on the real `show` output is `Ok`.
- `verify_profile_rejects_missing_and_zero`: given the text `  msx_vint:\n    Hash: 0x0baa90d19a7703b5\n    Counters: 7\n    Block counts: [0, 0, 0, 0, 0, 0, 0]\n`, returns `Err` naming `msx_vint`; given text with no `msx_vint` entry, returns `Err`; given the same text with `Block counts: [0, 13253300, 0, 0, 0, 0, 0]`, returns `Ok`.
- `r3copy_matches_r3_recipe`: `build_a(R3Copy)` succeeds; the disassembly of `msx_vint` in `r3copy` equals that in `r3`.
- `asm_candidate_bad_syntax`: `build_asm_candidate("vint", fixtures/bad_syntax.s, tmp)` is `Err` with `stage == "assemble"` and non-empty `stderr`.
- `rust_candidate_builds_both_recipes`: `selftest/vint/control_copy.rs` (the verbatim `decode_unsigned` logic and shim, written against `core` only) builds with R2 and R3; both outputs export `msx_vint` at alignment 0; neither has a `NEEDED` entry (`llvm-readobj --needed-libs`).
- `rust_candidate_panic_crashes`: a candidate whose body is `panic!()` builds, and calling it in a forked child dies with `SIGSEGV`.

Run: `msx_cargo dev test -p msx-harness --test build` — expected: fails.

- [ ] **Step 2: Implement `build.rs`, the wrapper, `msx rungs build` and `msx __train`**

`msx rungs build` builds R1, R2, R3, R3Copy in that order and prints `{"built": {"r1": "<sha256>", …}, "r3_differs_from_r2": {"vint": true, …}}`. `r3_differs_from_r2` compares each kernel's disassembly between the two rungs; it is information for the write-up, not a failure.

- [ ] **Step 3: Run the tests**

Run: `msx_cargo dev test -p msx-harness --test build` — expected: all pass. If the `staticlib` + `-nostdlib` link leaves undefined symbols for the trivial candidate, add the missing compiler-builtins archive from the sysroot to the link line; do not add `libc`.

- [ ] **Step 4: Commit**

```bash
git add experiment && git commit -m "feat: rung builds with verified PGO, candidate builds"
```

---

### Task 7: Static lint and differential checker

**Files:**
- Create: `experiment/harness/src/lint.rs`, `src/check.rs`
- Create: `experiment/selftest/vint/{control_ok.s,overread1.s,underread1.s,wrong_fe.s,writes_on_error.s,clobber_rbx.s,loop_ff.s,syscall.s,ctor.s,bigrodata.s,indirect.s,calls_out.s}`
- Create: `experiment/selftest/vint/{unsafe_overread.rs,uses_asm.rs}`
- Modify: `src/bin/msx.rs` (`check`, `__check-child`)
- Test: `experiment/harness/tests/lint.rs`, `tests/check.rs`

**Interfaces:**
- Consumes: Tasks 5 and 6.
- Produces:

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug)] pub enum Arm { Asm, Rust }
#[derive(Clone, Copy, PartialEq, Eq, Debug)] pub enum Phase { Search, Final }

pub fn lint_source(arm: Arm, src: &Path) -> Vec<String>;                 // empty = pass
pub fn lint_object(arm: Arm, kernel: &str, so: &Path) -> Vec<String>;    // empty = pass

#[derive(serde::Serialize)]
pub struct CheckReport {
    pub verdict: String,                 // PASS | INCORRECT | OOB_OR_CRASH | ABI_VIOLATION | TIMEOUT
    pub cases_run: u64,
    pub first_failure: Option<Failure>,  // the shortest failing input in the first failing batch
    pub mismatches_in_batch: u64,
}
#[derive(serde::Serialize)]
pub struct Failure { pub kind: String, pub input_hex: String, pub aux: u64, pub placement: String,   // "end" | "start"
                     pub expected: OutRec, pub got: Option<OutRec>, pub signal: Option<i32>, pub abi: Option<AbiViolation> }

/// `seed_file` is the candidate source; used only in Phase::Final to derive the random seed.
pub fn check(kernel: &str, so: &Path, phase: Phase, seed_file: Option<&Path>) -> CheckReport;
```

**Lint rules.** Source, assembly arm: reject the directives `.include` and `.incbin`, and any `.section` other than `.text`, `.rodata` (and `.rodata.*`), and `.note.GNU-stack`. Source, Rust arm: reject the tokens `asm!`, `global_asm!`, `naked`, `include_bytes!`, `include_str!`, `include!`, `static mut`, `#[link`, `#![`, `std::`, `alloc::`, `extern crate`, and any `extern` other than in one `extern "C" fn msx_<kernel>` definition. `unsafe` and `core::arch::x86_64` intrinsics are allowed.

Object, both arms (use `llvm-readobj` and `llvm-objdump`; cite the offending line in each message):

| Rule | Limit |
|---|---|
| exports `msx_<kernel>` as a function with nonzero size | required |
| entry address % 64 | 0 |
| size of `msx_<kernel>` | ≤ `TEXT_MAX` (8,192) |
| total size of `.rodata*` sections | ≤ 4,096 (asm), ≤ 16,384 (Rust) |
| file size | ≤ 262,144 bytes |
| `NEEDED` libraries | none |
| undefined symbols | asm: none. Rust: only `memcpy`, `memmove`, `memset`, `memcmp`, `bcmp` (the compiler emits these; the C library already loaded in the process resolves them, exactly as for the `A` builds) |
| dynamic relocations | asm: none. Rust: only `R_X86_64_GLOB_DAT` or `R_X86_64_JUMP_SLOT` against those five symbols |
| sections `.init`, `.fini`, `.init_array`, `.fini_array`, `.preinit_array`, `.ctors`, `.dtors` | absent |
| writable data sections (`.data`, `.bss`, `.tdata`, `.tbss`) with nonzero size | absent |
| instructions anywhere in executable sections: `syscall`, `sysenter`, `int`, `rdtsc`, `rdtscp`, `rdpmc`, `rdrand`, `rdseed`, `cpuid`, `xgetbv` | absent |
| indirect `call` or `jmp` (register or memory operand) | absent, except in the Rust arm a `call`/`jmp` through the GOT or PLT slot of one of those five symbols |
| direct `call`/`jmp` whose target is outside the file's executable sections | absent, with the same Rust-arm exception |
| asm arm only: any `call` | absent (leaf only) |

**Checker design** (decisions the signature does not determine):

- Cases are produced in a fixed order: `edge_cases`, then `random_cases(seed, n)`, then `trace_cases` of every segment in the phase's split (`search` for `Phase::Search`, `holdout` for `Phase::Final`). Seed and `n` come from the constants table: Search uses `0x5EED0001` and 2,000,000; Final uses the first 8 bytes, little-endian, of SHA-256 of `seed_file` and 20,000,000.
- Every case runs twice: placed with `place_end` and with `place_start`.
- Cases are processed in batches of 1,000,000. For each batch the parent starts `msx __check-child` (a fresh process, same binary). The child and the parent **both** regenerate the batch deterministically from `(kernel, phase, seed, batch index)`; no inputs cross the process boundary.
- The child loads `A.r1` (only to enumerate trace cases) and the candidate, sets `RLIMIT_CPU` to 240 s and `RLIMIT_CORE` to 0, maps a shared file under `work/tmp/` containing a progress counter followed by one result slot per run (`OutRec` plus an ABI-violation byte), stores the index of each run in the progress counter **before** making the call, and writes the result after.
- The parent waits with a wall-clock limit of `CHECK_TIMEOUT_SECS` per batch (overridable for tests with the environment variable `MSX_CHECK_TIMEOUT_SECS`). Outcomes:
  - child killed by the timeout or by `SIGXCPU` → `TIMEOUT`, failure input = the run at the progress counter;
  - child killed by any other signal → `OOB_OR_CRASH`, with the signal number and the run at the progress counter;
  - otherwise the parent computes the reference `OutRec` for every run by calling `A.r1` in its own process and compares: any ABI byte set → `ABI_VIOLATION`; any `OutRec` difference → `INCORRECT`.
- The check stops at the first failing batch. `first_failure` is the failing run with the shortest input in that batch.
- `msx check` also accepts `--so` pointing at an `A` build; that is how rung agreement is verified in Task 11.

**Sabotage and control candidates for `vint`** (hand-written; each is a complete candidate file). These are the checker's own tests and become the self-test in Task 10:

| File | What it does | Expected |
|---|---|---|
| `control_ok.s` | correct, simple: lead byte, `lzcnt` of the inverted byte, byte loop | lint pass, `PASS` |
| `overread1.s` | correct results, but when `len ≥ 1` always loads 8 bytes starting at `ptr + 1` | `OOB_OR_CRASH` |
| `underread1.s` | correct results, but reads the byte at `ptr − 1` | `OOB_OR_CRASH` |
| `wrong_fe.s` | correct except lead byte `0xFE` returns value + 1 | `INCORRECT`, failure input starts with `fe` |
| `writes_on_error.s` | correct, but writes 0 to `*out` on the truncated path | `INCORRECT` |
| `clobber_rbx.s` | correct, but uses `rbx` as scratch without saving | `ABI_VIOLATION` |
| `loop_ff.s` | spins forever on lead byte `0xFF` | `TIMEOUT` |
| `syscall.s` | contains a `syscall` instruction on an unreachable path | lint fail |
| `ctor.s` | has an `.init_array` entry | lint fail |
| `bigrodata.s` | carries an 8,192-byte `.rodata` table | lint fail |
| `indirect.s` | dispatches through a jump table with `jmp *(%r…)` | lint fail |
| `calls_out.s` | calls `memcpy@PLT` | lint fail (or `BUILD`-stage link failure; either is accepted, recorded which) |
| `unsafe_overread.rs` | Rust: `ptr.add(1).cast::<u64>().read_unaligned()` regardless of `len` | lint pass, `OOB_OR_CRASH` |
| `uses_asm.rs` | Rust: contains `core::arch::asm!` | lint fail |

- [ ] **Step 1: Write the failing tests**

`tests/lint.rs`: one test per lint-fail row above asserting a non-empty error list whose text names the rule (`syscall`, `.init_array`, `.rodata`, `indirect`, `asm!`); `control_ok.s`, `control_copy.rs` and `unsafe_overread.rs` produce empty lists from both `lint_source` and `lint_object`.

`tests/check.rs` (set `MSX_CHECK_TIMEOUT_SECS=10`): one test per non-lint row asserting the expected verdict with `Phase::Search`; plus
- `overread_is_caught_at_every_length`: for `overread1.s`, the failure's input length is at most 8 (the over-read is caught on short inputs, not only on long ones);
- `wrong_fe_reports_shortest`: the failure input for `wrong_fe.s` is exactly 8 bytes long;
- `final_seed_depends_on_file`: `check(.., Phase::Final, Some(a))` and `Some(b)` for two different files derive different seeds (expose the seed derivation as a pure function and test that);
- `a_r1_passes_against_itself`: `check("vint", A.r1, Search, None)` is `PASS` with `cases_run` equal to 2 × (33,280 + 2,000,000 + manifest search ops).

Run: `msx_cargo dev test -p msx-harness --test lint --test check` — expected: fails.

- [ ] **Step 2: Write the sabotage and control candidates**
- [ ] **Step 3: Implement `lint.rs`**
- [ ] **Step 4: Implement `check.rs`, `msx check`, `msx __check-child`**
- [ ] **Step 5: Run the tests**

Run: `msx_cargo dev test -p msx-harness --test lint --test check` — expected: all pass. If a sabotage candidate is not caught, the fix belongs in the checker or the case generators, never in the expectation.

- [ ] **Step 6: Commit**

```bash
git add experiment && git commit -m "feat: static lint and differential checker with sabotage tests"
```

---

### Task 8: Counters, measurement sessions, statistics

**Files:**
- Create: `experiment/harness/src/counters.rs`, `src/measure.rs`, `src/stats.rs`
- Modify: `src/bin/msx.rs` (`measure`, `__session`)
- Test: `experiment/harness/tests/stats.rs`, `tests/hygiene.rs`, `tests/measure_smoke.rs`

**Interfaces:**
- Consumes: Tasks 5 and 6.
- Produces:

```rust
// counters.rs
pub struct Reading { pub cycles: u64, pub instructions: u64, pub branch_misses: u64, pub time_enabled: u64, pub time_running: u64 }
pub struct Counters { /* one perf_event_open group for the calling thread */ }
impl Counters { pub fn open() -> Result<Counters, String>; pub fn read(&self) -> Reading; }

// measure.rs
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Contestant { pub name: String, pub so: PathBuf, pub symbol: String, pub is_chain: bool }  // is_chain: call msx_<k>_chain directly (A.inline)
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum Discard { Preempted, Multiplexed, InstrMismatch }
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Sample { pub session: u32, pub block: u32, pub chunk: u32, pub contestant: String, pub ops: u64,
                    pub cycles: u64, pub instructions: u64, pub branch_misses: u64, pub wall_ns: u64, pub discard: Option<Discard> }
#[derive(Clone, Copy, PartialEq, Eq, Debug)] pub enum Protocol { Search, Final }
#[derive(Clone, Copy, PartialEq, Eq, Debug)] pub enum Split { Search, Holdout }

pub fn apply_hygiene(samples: &mut [Sample]);                       // sets InstrMismatch; marks whole blocks
pub fn session_is_valid(samples: &[Sample]) -> bool;                // discarded blocks ≤ MAX_DISCARD_FRACTION
pub fn run_protocol(kernel: &str, contestants: &[Contestant], protocol: Protocol, split: Split) -> Result<RunResult, String>;
        // Err("NOISE_GATE_FAILED: …") after MAX_SESSION_RERUNS

// stats.rs
#[derive(Clone, Copy, Debug, serde::Serialize)] pub struct Interval { pub point: f64, pub lo: f64, pub hi: f64 }
pub fn session_ratio(samples: &[Sample], cand: &str, base: &str) -> Option<f64>;
pub fn bootstrap_ci(session_ratios: &[f64]) -> Interval;
pub fn null_band(nulls: &[Interval]) -> f64;
pub fn threshold(band: f64) -> f64;
pub fn verdict(iv: Interval, t: f64) -> &'static str;
```

`RunResult` serializes to the `measure` JSON: per-contestant medians across valid sessions of cycles/op, ns/op, instructions/op and branch-misses/op; every pairwise `Interval` requested; and per-session metadata (core, sibling utilization, start and end frequency, discarded fraction, reruns, environment pad length, load order).

**Counters:** one group led by `PERF_COUNT_HW_CPU_CYCLES`, with `PERF_COUNT_HW_INSTRUCTIONS` and `PERF_COUNT_HW_BRANCH_MISSES`; `exclude_kernel = 1`, `exclude_hv = 1`, `pid = 0`, `cpu = -1`; `read_format = PERF_FORMAT_GROUP | TOTAL_TIME_ENABLED | TOTAL_TIME_RUNNING`. Enabled once at open; a sample is the difference of two `read`s. This configuration was verified on this host with a C probe (README, "Verified on this host"); if `open` fails now, something changed: stop and report (gate 3).

**Run procedure (`run_protocol`, in the parent):**
1. Check `/proc/sys/kernel/randomize_va_space` is 2; record it. If it is not, record a warning in the result; do not change it.
2. Choose the core once per run: among physical cores 2..=15, sample `/proc/stat` over 500 ms and take the core whose own plus SMT-sibling (`core + 16`) utilization is lowest. If per-CPU lines are unavailable, use core 5 and record `core_choice: "fixed"`.
3. For each session index, start `msx __session` as a fresh process with: the core; a session seed (`0x6D7378` + session index + 1000 × rerun count); an environment variable `MSX_PAD` of random length 0..=4095 bytes; the contestant list as JSON.
4. Collect the session's samples. Read the sibling's utilization over the session from `/proc/stat`; above 20% the session is invalid. Apply `apply_hygiene`; if `session_is_valid` is false the session is invalid. Rerun an invalid session, up to `MAX_SESSION_RERUNS` reruns in total for the run; beyond that return `NOISE_GATE_FAILED`.

**Session procedure (`msx __session`, in the child):**
1. Pin to the core with `sched_setaffinity`.
2. Load `A.r1` first as the reference for chunking (it is never unloaded). Load the contestants in an order shuffled by the session seed; between loads, `mmap` and keep a mapping of random size 0..=255 pages.
3. Build the chunk list from the split's segments with `chunks(reference, seg, SAMPLE_MIN_OPS)`. Choose `D = min(number of chunks, blocks / 3)` distinct chunks by seeded shuffle. Block `i` uses chunk `i mod D`.
4. Compute each chosen chunk's reference `(ops, checksum)` once with `A.r1`.
   **Small-trace mode** (when the manifest's `small_trace.<split>` is true): there are no chunks. A sample's input is the split's whole segment list, visited in an order shuffled freshly for each block (seeded by session seed and block index; the same order for every contestant in the block), repeated `R` times with a new shuffle each pass, where `R` is the smallest count giving at least `SAMPLE_MIN_OPS` operations. `run_chain` is called once per segment and the operations and checksums are summed. Every sample then does identical work, so the instruction-count check uses one mode per contestant for the whole session. The result records `inputs_repeat: true` and `R`.
5. Warm up: every contestant runs the first chosen chunk (or, in small-trace mode, one pass over the split) three times, unmeasured.
6. For each block: for each contestant in a seeded shuffled order: touch every byte of the chunk with a plain summing loop (cache state equalized without training the kernel's branches); record `getrusage(RUSAGE_THREAD)` involuntary switches, `CLOCK_MONOTONIC_RAW`, and a counter `read`; run `run_chain` once over the chunk (or call the chain symbol when `is_chain`); read all three again. Mark `Preempted` if involuntary switches increased, `Multiplexed` if the `time_enabled` and `time_running` deltas differ. If `(ops, checksum)` differs from the reference, abort the whole run with a tool error: a contestant diverged during measurement.
7. Print all samples as JSON.

**Hygiene (`apply_hygiene`):** for each (contestant, chunk) take the modal instruction count across that session's samples; mark `InstrMismatch` on any sample differing from the mode by more than `INSTR_TOLERANCE` (relative); if there is no mode (all differ), mark them all. Then mark every sample of any block that contains a discarded sample.

**Statistics:**
- `session_ratio`: over blocks with no discard, the median of `(cand cycles / cand ops) / (base cycles / base ops)`. `None` if fewer than 10 valid blocks.
- `bootstrap_ci`: `point` is the geometric mean of the session ratios; `lo`/`hi` are the 2.5th and 97.5th percentiles of the geometric mean over `BOOTSTRAP_RESAMPLES` resamples with replacement, ChaCha RNG seeded `0x6D7378`.
- `null_band`: the maximum over the given intervals of `max(1 − lo, hi − 1)`.
- `threshold`: `max(THRESHOLD_FLOOR, 2 × band)`.
- `verdict`: the table in spec section 7.2.

Protocol sizes come from the constants table. For tests only, the environment variables `MSX_SESSIONS` and `MSX_BLOCKS` override them; the result records the sizes actually used, and Task 9's commands refuse to write a ledger record of kind `attempt`, `null` or `final` when an override is active.

- [ ] **Step 1: Write the failing tests**

`tests/stats.rs`:
- `bootstrap_of_constant`: `bootstrap_ci(&[0.9; 30])` has `point`, `lo`, `hi` all equal to 0.9 within 1e-12.
- `bootstrap_is_deterministic`: two calls on the same 30 distinct ratios return identical intervals, with `lo < point < hi`.
- `verdict_table`, with `t = 0.05`: `(lo 0.90, hi 0.94)` → `WON`; `(0.96, 1.03)` → `TIED`; `(1.06, 1.20)` → `LOST`; `(0.93, 0.97)` → `INCONCLUSIVE`; `(0.94, 1.06)` → `INCONCLUSIVE`.
- `threshold_rule`: `threshold(0.01) == 0.05`; `threshold(0.04) == 0.08`.
- `null_band_rule`: for intervals `(0.99, 1.02)` and `(0.97, 1.00)` the band is 0.03.
- `session_ratio_median`: synthetic samples for two contestants over 11 blocks with cycles/op ratios 0.5, 0.9 × 9, 2.0 → 0.9; with only 9 valid blocks → `None`.

`tests/hygiene.rs` (synthetic `Sample`s, no hardware):
- a `Preempted` sample discards its whole block and no other;
- a sample whose instruction count is 0.1% off the mode is marked `InstrMismatch`; one 0.001% off is kept;
- 31 discarded blocks out of 100 → `session_is_valid` is false; 30 out of 100 → true.

`tests/measure_smoke.rs` (hardware; `MSX_SESSIONS=2 MSX_BLOCKS=30`):
- copy `A.r1`'s library to a second path and measure `A.r1` against its copy for `vint`: the run succeeds, every session has at least 10 valid blocks, instructions/op is identical for the two contestants within 0.01%, and the ratio's `point` lies in 0.8..=1.25.
- `Counters::open()` succeeds and two consecutive reads show `cycles` increasing.

Run: `msx_cargo dev test -p msx-harness --test stats --test hygiene --test measure_smoke` — expected: fails.

- [ ] **Step 2: Implement `stats.rs`** — run `--test stats`, expected pass.
- [ ] **Step 3: Implement `counters.rs` and `measure.rs`, `msx measure`, `msx __session`** — run `--test hygiene --test measure_smoke`, expected pass.
- [ ] **Step 4: Commit**

```bash
git add experiment/harness && git commit -m "feat: in-process counters, measurement sessions and statistics"
```

---

### Task 9: Ledger and the `attempt`, `null`, `final` commands

**Files:**
- Create: `experiment/harness/src/ledger.rs`, `src/commands.rs`
- Modify: `src/bin/msx.rs`
- Test: `experiment/harness/tests/ledger.rs`, `tests/commands.rs`

**Interfaces:**
- Consumes: Tasks 5-8.
- Produces: the `attempt`, `null`, `final` commands of the README CLI table, and

```rust
// ledger.rs
pub fn append(record: serde_json::Value) -> Result<String /* id */, String>;   // adds id, ts, git_head, git_dirty, host, tool_sha256, a_sha256
pub fn read_all() -> Vec<serde_json::Value>;
pub struct Budget { pub submissions: u32, pub counted: u32 }
pub fn budget(records: &[serde_json::Value], kernel: &str, arm: &str) -> Budget;

// commands.rs
pub fn tree_is_clean_for(porcelain: &str, kernel: &str, arm: &str) -> Result<(), Vec<String>>;   // offending paths
pub fn parse_final_md(text: &str) -> Option<u32>;                                                // "FINAL: attempt-07" -> 7
```

**Ledger record.** One JSON object per line in `experiment/results/ledger.jsonl`. Common fields added by `append`:

| Field | Value |
|---|---|
| `id` | `L` + six-digit sequence number, one more than the last line's |
| `ts` | UTC, RFC 3339 |
| `git_head`, `git_dirty` | `git rev-parse HEAD`; whether `git status --porcelain` is non-empty |
| `host` | CPU model string, kernel release, governor of the measurement core, 1-minute load average |
| `tool_sha256` | SHA-256 of the running `msx` binary |
| `a_sha256` | SHA-256 of each existing `A` library, keyed `r1`, `r2`, `r3`, `r3copy` |

Command-specific fields: `kind` (`attempt`, `refused`, `null`, `final`, `selftest`), `kernel`, `arm`, `agent_id` and `model` (from the environment variables `MSX_AGENT_ID` and `MSX_MODEL`, required for `attempt`), `submission`, `counted_attempt` (number or null), `candidate_path`, `candidate_sha256`, `lint` (error list), `build` (stage and stderr on failure), `check` (`CheckReport`), `measure` (`RunResult`), `protocol_sizes`.

Appends take an exclusive `flock` on `experiment/results/ledger.lock`. The commands below hold an exclusive `flock` on `work/msx.lock` from start to finish, so only one build-check-measure sequence runs on the host at a time; a second invocation blocks and waits.

**`msx attempt --kernel K --arm asm|rust --file PATH`**, in order:

1. Refuse with a tool error (exit 2, no ledger record) if `MSX_AGENT_ID` or `MSX_MODEL` is unset, or if a size override from Task 8 is active.
2. Tree check: `tree_is_clean_for(git status --porcelain, K, arm)`. Anything changed or untracked outside `experiment/candidates/K/<arm>/` and `experiment/results/ledger.jsonl` is a tool error listing the paths.
3. File check: `PATH` is inside `experiment/candidates/K/<arm>/`, named `attempt-NN.s` (asm) or `attempt-NN.rs` (Rust) where `NN` is this agent's next submission number, two digits; its SHA-256 is not already in the ledger for this kernel and arm.
4. Budget: if `submissions ≥ MAX_SUBMISSIONS` or `counted ≥ COUNTED_ATTEMPTS`, append a `refused` record and print `{"verdict":"BUDGET_EXHAUSTED"}`.
5. `lint_source` → `LINT_FAILED`. Build → `BUILD_FAILED`. `lint_object` → `LINT_FAILED`. These are free: `counted_attempt` is null.
6. `check(K, cand.so, Phase::Search, None)`. This attempt is now counted, whatever the verdict.
7. On `PASS`: `run_protocol(K, [A.r1, A.r2, A.r3, cand], Protocol::Search, Split::Search)`, with intervals for `cand` against each rung.
8. Append the record. Print:

```json
{ "submission": 7, "counted_attempt": 5, "counted_attempts_left": 5, "submissions_left": 23,
  "verdict": "PASS",
  "lint_errors": [], "build_stderr": null, "first_failure": null,
  "timing": { "provisional": true,
              "cycles_per_op": { "A.r1": 0.0, "A.r2": 0.0, "A.r3": 0.0, "cand": 0.0 },
              "instructions_per_op": { }, "branch_misses_per_op": { }, "ns_per_op": { },
              "ratio_vs": { "A.r1": { "point": 0.0, "lo": 0.0, "hi": 0.0 }, "A.r2": { }, "A.r3": { } } } }
```

`timing` is null unless the verdict is `PASS`. If the measurement returns `NOISE_GATE_FAILED`, the record says so, `timing` is null, and the attempt **does not count** (set `counted_attempt` to null): the agent may resubmit the same file under the next submission number, and step 3's duplicate-hash rule is waived for a file whose only prior record is a noise failure.

**`msx null --kernel K`:** `run_protocol(K, [A.r3, A.r3copy, A.reasm if work/build/A/reasm/K.so exists], Protocol::Final, Split::Search)`. Output and ledger record contain each null `Interval`, the `band`, and `pass: band ≤ NULL_BAND_MAX`. Also written to `experiment/results/null/K-<ledger id>.json`.

**`msx final --kernel K [--rerun-reason TEXT]`:**

1. Refuse if the ledger already holds a `final` record for `K` with a measurement, unless `--rerun-reason` is given; the reason is stored in the new record. A rerun is legitimate only after a tool failure or `NOISE_GATE_FAILED`.
2. For each arm read `experiment/candidates/K/<arm>/FINAL.md`; `parse_final_md` gives the chosen attempt. The arm's status is `NO_ENTRY` if the file is missing, unparsable, or names an attempt whose ledger record is not a `PASS`.
3. For each entered arm: `check(K, cand.so, Phase::Final, Some(candidate source))`. Anything but `PASS` → status `DISQUALIFIED`, recorded with the report.
4. Build `cand-r2.so` for a surviving Rust finalist.
5. `run_protocol(K, contestants, Protocol::Final, Split::Holdout)` with contestants: `A.r1`, `A.r2`, `A.r3`, `A.r3copy`, `A.reasm` (if it exists), `A.inline` (`A.r3`'s library, `msx_K_chain`, `is_chain = true`), `B.asm`, `B.rust` (`cand.so`), `B.rust.r2`: omitting those not available.
6. Compute: nulls `A.r3copy / A.r3` and `A.reasm / A.r3`; `band`; `T = threshold(band)`. If `band > NULL_BAND_MAX` the whole result is `NOISE_GATE_FAILED` and no verdicts are issued.
7. Otherwise issue a verdict, with its `Interval`, for: each `B` against each of `A.r1`, `A.r2`, `A.r3`; `B.asm` against `B.rust`; `A.r2` against `A.r1` and `A.r3` against `A.r2` (the ladder's own steps); and report the boundary tax `A.r3 / A.inline` as an `Interval` without a verdict.
8. Append the record; write `experiment/results/final/K.json`.

- [ ] **Step 1: Write the failing tests**

`tests/ledger.rs` (ledger path overridden to a temp file by the environment variable `MSX_LEDGER`):
- two appends produce ids `L000001`, `L000002`; `read_all` returns both in order with all common fields present;
- `budget` over a synthetic ledger with 4 free failures, 3 counted failures, 2 counted passes and 1 noise failure for `(vint, asm)` and unrelated records for `(vint, rust)` returns `submissions == 10`, `counted == 5`.

`tests/commands.rs`:
- `tree_is_clean_for(" M experiment/candidates/vint/asm/attempt-03.s\n?? experiment/candidates/vint/asm/NOTES.md\n M experiment/results/ledger.jsonl\n", "vint", "asm")` is `Ok`;
- the same plus ` M experiment/harness/src/check.rs` is `Err` containing that path; plus `?? experiment/candidates/vint/rust/x.rs` is `Err`;
- `parse_final_md("FINAL: attempt-07\nwhy…")` is `Some(7)`; `"final: 7"` is `None`; empty is `None`;
- `attempt_refuses_without_identity`: running `msx attempt …` with `MSX_AGENT_ID` unset exits 2 and appends nothing.

Run: `msx_cargo dev test -p msx-harness --test ledger --test commands` — expected: fails.

- [ ] **Step 2: Implement `ledger.rs`**
- [ ] **Step 3: Implement `commands.rs` and wire the three commands**
- [ ] **Step 4: Run the tests** — expected: all pass.
- [ ] **Step 5: Commit**

```bash
git add experiment/harness && git commit -m "feat: ledger and attempt/null/final commands"
```

---

### Task 10: Self-test: sabotage and control candidates end to end

**Files:**
- Create: `experiment/selftest/vint/control_slow.s`, `experiment/selftest/vint/EXPECT.json`
- Create: `experiment/selftest/index_framing/…`, `experiment/selftest/bti_sparse_find/…` (each: `control_ok.s`, `overread.s`, `wrong.s`, `control_slow.s`, `EXPECT.json`)
- Create: `experiment/selftest/reasm/<kernel>.s` where producible; `experiment/selftest/reasm/README.md`
- Modify: `src/commands.rs`, `src/bin/msx.rs` (`selftest`)
- Test: `experiment/harness/tests/selftest_expect.rs`

**Interfaces:**
- Consumes: everything in Phase 2.
- Produces: `msx selftest`, printing `{"pass": true|false, "results": [ { "kernel", "file", "expected", "got", "ok" } ]}` and appending one `selftest` ledger record. Produces `work/build/A/reasm/<kernel>.so`.

`EXPECT.json` maps each file to its expected outcome: a check verdict, `LINT_FAILED`, or for controls a measured verdict against `A.r3` from the search protocol on the search split:

| File | Expected |
|---|---|
| every sabotage file from Task 7 | the verdict in Task 7's table |
| `control_ok.s` | check `PASS`; measured verdict recorded, any value accepted |
| `control_slow.s` (`control_ok.s` plus a dependent chain of 24 `imul` instructions on the hot path) | check `PASS`; measured `LOST` |
| `control_copy.rs` (Rust path; also the stand-in for the missing `vint` reasm) | check `PASS`; measured `TIED` |
| `reasm/<kernel>.s` | lint pass, check `PASS`; measured `TIED` |

- [ ] **Step 1: Produce `A.reasm` for each kernel**

`vint` has no `A.reasm`: its compiler output calls `memcpy`, an external symbol the candidate rules forbid (verified 12 November 2026). Record that in `experiment/selftest/reasm/README.md` and skip it. Its null is `A.r3copy` alone, and its Rust-path control is `control_copy.rs`.

For `index_framing` and `bti_sparse_find`, take `A.r3`'s assembly for `msx_<kernel>`: build the kernels crate with the `A.r3` flags plus `--emit=asm` into `work/target/A-r3-asm`, and cut out the `msx_<kernel>` function with the local labels and constants it references into a standalone candidate file, `experiment/selftest/reasm/<kernel>.s`, with `.p2align 6` before the entry label. Change nothing else. If the function references any symbol outside itself, `A.reasm` is not producible for that kernel: say so in `experiment/selftest/reasm/README.md` and omit the file; the commands already treat it as optional.

Build each with `build_asm_candidate` into `work/build/A/reasm/` and rename the output `<kernel>.so`. Confirm the bytes of `msx_<kernel>` equal those in `A.r3` (`llvm-objdump -d` of both, addresses stripped). A difference means the extraction changed the code: fix the extraction.

- [ ] **Step 2: Write the `index_framing` and `bti_sparse_find` sabotage and control files**

Per kernel: one correct simple implementation, one that over-reads past `ptr + len` while returning correct results, one that returns a wrong value on one structural case named in the file's first comment, and the slow control.

- [ ] **Step 3: Write the failing test**

`tests/selftest_expect.rs`: every file under `experiment/selftest/<kernel>/` and `experiment/selftest/reasm/` has an entry in an `EXPECT.json`, and every entry names an existing file. Expected: fails until the files exist.

- [ ] **Step 4: Implement `msx selftest`**

It runs lint, build and `check(Phase::Search)` for every entry, and for controls a search-protocol measurement against `A.r3` with `T = THRESHOLD_FLOOR`. It does not use `msx attempt` and consumes no agent budget.

- [ ] **Step 5: Run it**

```bash
. experiment/env.sh && msx_cargo tool build --release -p msx-harness --manifest-path experiment/Cargo.toml
work/target/tool/release/msx rungs build
work/target/tool/release/msx selftest
```

Expected: `"pass": true`. This binary, `work/target/tool/release/msx`, is the one used for everything in Phase 3.

What a failure means:
- a sabotage file not caught → the checker has a hole; fix it and add a test in Task 7's suite;
- `control_slow.s` not `LOST` → the measurement cannot see a real slowdown; investigate before continuing;
- `control_copy.rs` or a `reasm` file not `TIED` → the ruler shows a difference between identical code. Run it twice more. If it persists, record the three results and continue to Task 11, whose gate decides.

- [ ] **Step 6: Commit**

```bash
git add experiment && git commit -m "feat: end-to-end self-test with sabotage and control candidates"
```
