//! The differential checker: judges a candidate shared object against `A.r1`.
//!
//! The candidate runs in a child process (`msx __check-child`); the parent regenerates the same
//! cases, computes the reference answers in its own process and compares. A misbehaving
//! candidate can therefore crash or hang only the child.

use crate::build::{a_path, build_a, work, Rung};
use crate::guard::GuardBuf;
use crate::loader::Lib;
use crate::spec::{kernel, load_segment, manifest_segments, traces_dir, KernelSpec, OutRec, RawFn};
use crate::tramp::AbiViolation;
use sha2::{Digest, Sha256};
use std::os::unix::process::ExitStatusExt;
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

// ---- protocol constants (plan: "Protocol constants") ----

pub const RANDOM_CASES_SEARCH: u64 = 2_000_000;
pub const SEED_SEARCH: u64 = 0x5EED_0001;
pub const RANDOM_CASES_FINAL: u64 = 20_000_000;
/// Wall-clock limit per batch.
pub const CHECK_TIMEOUT_SECS: u64 = 300;
/// Child CPU rlimit (soft; the hard limit is a few seconds above so SIGXCPU is delivered first).
pub const CHILD_CPU_SECS: u64 = 240;
/// Assembly candidates: `.rodata*` bytes.
pub const RODATA_MAX_ASM: u64 = 4_096;
/// Rust candidates: `.rodata*` bytes.
pub const RODATA_MAX_RUST: u64 = 16_384;
/// Bytes of code in the kernel function.
pub const TEXT_MAX: u64 = 8_192;
/// Candidate shared object file size.
pub const FILE_MAX: u64 = 262_144;
/// Cases per child process.
pub const BATCH_CASES: u64 = 1_000_000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Search,
    Final,
}

#[derive(serde::Serialize, Debug, Clone)]
pub struct CheckReport {
    /// PASS | INCORRECT | OOB_OR_CRASH | ABI_VIOLATION | TIMEOUT
    pub verdict: String,
    /// Individual runs (cases x 2 placements) executed.
    pub cases_run: u64,
    /// The shortest failing input in the first failing batch.
    pub first_failure: Option<Failure>,
    /// Failing runs in that batch (0 for crashes and timeouts, which end the batch).
    pub mismatches_in_batch: u64,
}

#[derive(serde::Serialize, Debug, Clone)]
pub struct Failure {
    pub kind: String,
    pub input_hex: String,
    pub aux: u64,
    /// "end" | "start"
    pub placement: String,
    pub expected: OutRec,
    pub got: Option<OutRec>,
    pub signal: Option<i32>,
    pub abi: Option<AbiViolation>,
}

/// First 8 bytes, little-endian, of SHA-256 of the candidate source.
pub fn seed_from_source(bytes: &[u8]) -> u64 {
    let d = Sha256::digest(bytes);
    u64::from_le_bytes(d[..8].try_into().unwrap())
}

/// (seed, random case count) for a phase.
pub fn phase_params(phase: Phase, seed_file: Option<&Path>) -> Result<(u64, u64), String> {
    match phase {
        Phase::Search => Ok((SEED_SEARCH, RANDOM_CASES_SEARCH)),
        Phase::Final => {
            let f = seed_file.ok_or("Phase::Final needs the candidate source file as seed_file")?;
            let bytes = std::fs::read(f).map_err(|e| format!("{}: {e}", f.display()))?;
            Ok((seed_from_source(&bytes), RANDOM_CASES_FINAL))
        }
    }
}

fn split_of(phase: Phase) -> &'static str {
    match phase {
        Phase::Search => "search",
        Phase::Final => "holdout",
    }
}

fn phase_name(phase: Phase) -> &'static str {
    match phase {
        Phase::Search => "search",
        Phase::Final => "final",
    }
}

// ---- case generation (shared by parent and child; fully deterministic) ----

/// Edge cases, then `n` random cases, then every trace case of the split's segments.
/// A kernel without a traces directory contributes no trace cases.
fn generate(k: &dyn KernelSpec, reference: RawFn, split: &str, seed: u64, n: u64, sink: &mut dyn FnMut(&[u8], u64)) -> Result<(), String> {
    k.edge_cases(sink);
    k.random_cases(seed, n, sink);
    if traces_dir().join(k.name()).join("MANIFEST.json").is_file() {
        for seg in manifest_segments(k.name(), Some(split))? {
            let buf = load_segment(&seg.path).map_err(|e| format!("{}: {e}", seg.path.display()))?;
            unsafe { k.trace_cases(reference, &buf, sink) };
        }
    }
    Ok(())
}

struct StopGeneration;

/// Calls `f(index_in_batch, input, aux)` for each case of batch `batch`, then stops generating.
/// Returns the number of cases in the batch.
fn for_batch(
    k: &dyn KernelSpec,
    reference: RawFn,
    split: &str,
    seed: u64,
    n: u64,
    batch: u64,
    f: &mut dyn FnMut(usize, &[u8], u64),
) -> Result<u64, String> {
    let (start, end) = (batch * BATCH_CASES, (batch + 1) * BATCH_CASES);
    let mut g = 0u64;
    let r = catch_unwind(AssertUnwindSafe(|| {
        generate(k, reference, split, seed, n, &mut |inp, aux| {
            if g >= end {
                resume_unwind(Box::new(StopGeneration));
            }
            if g >= start {
                f((g - start) as usize, inp, aux);
            }
            g += 1;
        })
    }));
    match r {
        Ok(res) => res?,
        Err(p) if p.is::<StopGeneration>() => {}
        Err(p) => resume_unwind(p),
    }
    Ok(g.min(end).saturating_sub(start))
}

fn count_cases(k: &dyn KernelSpec, reference: RawFn, split: &str, seed: u64, n: u64) -> Result<u64, String> {
    let mut c = 0u64;
    generate(k, reference, split, seed, n, &mut |_, _| c += 1)?;
    Ok(c)
}

// ---- guard buffers ----

const SMALL: usize = 4096;

struct Bufs {
    small: GuardBuf,
    big: Option<(GuardBuf, usize)>,
}

impl Bufs {
    fn new() -> Bufs {
        Bufs { small: GuardBuf::new(SMALL), big: None }
    }

    fn place(&mut self, input: &[u8], end: bool) -> *const u8 {
        let buf = if input.len() <= SMALL {
            &mut self.small
        } else {
            if self.big.as_ref().is_none_or(|(_, cap)| *cap < input.len()) {
                let cap = input.len().next_power_of_two();
                self.big = Some((GuardBuf::new(cap), cap));
            }
            &mut self.big.as_mut().unwrap().0
        };
        if end {
            buf.place_end(input)
        } else {
            buf.place_start(input)
        }
    }
}

// ---- shared result file ----

const HEADER: usize = 64; // [0] progress (run index), [8] cases finished
const SLOT: usize = 48; // OutRec (40 bytes), ABI byte, padding

fn abi_code(v: AbiViolation) -> u8 {
    match v {
        AbiViolation::CalleeSaved => 1,
        AbiViolation::StackPointer => 2,
        AbiViolation::DirectionFlag => 3,
        AbiViolation::Mxcsr => 4,
        AbiViolation::UpperVectorState => 5,
    }
}

fn abi_from_code(c: u8) -> Option<AbiViolation> {
    Some(match c {
        1 => AbiViolation::CalleeSaved,
        2 => AbiViolation::StackPointer,
        3 => AbiViolation::DirectionFlag,
        4 => AbiViolation::Mxcsr,
        5 => AbiViolation::UpperVectorState,
        _ => return None,
    })
}

fn rec_to_bytes(r: &OutRec) -> [u8; 40] {
    let mut b = [0u8; 40];
    b[0..8].copy_from_slice(&r.ret.to_le_bytes());
    for i in 0..3 {
        b[8 + 8 * i..16 + 8 * i].copy_from_slice(&r.out[i].to_le_bytes());
    }
    b[32..40].copy_from_slice(&r.out_written.to_le_bytes());
    b
}

fn rec_from_bytes(b: &[u8]) -> OutRec {
    let w = |i: usize| u64::from_le_bytes(b[8 * i..8 * i + 8].try_into().unwrap());
    OutRec { ret: w(0), out: [w(1), w(2), w(3)], out_written: w(4) }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

// ---- child ----

fn arg_after(args: &[String], flag: &str) -> Option<String> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned()
}

fn need(args: &[String], flag: &str) -> Result<String, String> {
    arg_after(args, flag).ok_or_else(|| format!("{flag} required"))
}

/// `msx __check-child --kernel K --so PATH --phase P --seed N --n N --batch I --cases C --shm FILE`.
/// Exit code 3 means the candidate could not be loaded.
pub fn child_main(args: &[String]) -> Result<(), String> {
    unsafe {
        let cpu = libc::rlimit { rlim_cur: CHILD_CPU_SECS, rlim_max: CHILD_CPU_SECS + 5 };
        libc::setrlimit(libc::RLIMIT_CPU, &cpu);
        let core = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
        libc::setrlimit(libc::RLIMIT_CORE, &core);
    }
    let kname = need(args, "--kernel")?;
    let k = kernel(&kname).ok_or(format!("unknown kernel {kname}"))?;
    let so = PathBuf::from(need(args, "--so")?);
    let phase = match need(args, "--phase")?.as_str() {
        "search" => Phase::Search,
        "final" => Phase::Final,
        o => return Err(format!("bad phase {o}")),
    };
    let seed: u64 = need(args, "--seed")?.parse().map_err(|e| format!("--seed: {e}"))?;
    let n: u64 = need(args, "--n")?.parse().map_err(|e| format!("--n: {e}"))?;
    let batch: u64 = need(args, "--batch")?.parse().map_err(|e| format!("--batch: {e}"))?;
    let cases: u64 = need(args, "--cases")?.parse().map_err(|e| format!("--cases: {e}"))?;
    let shm_path = PathBuf::from(need(args, "--shm")?);

    let ref_lib = Lib::open(&a_path(Rung::R1))?;
    let reference = ref_lib.sym(k.symbol()).ok_or("A.r1 lacks the kernel symbol")?;
    let cand_lib = match Lib::open(&so) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("msx check-child: cannot load candidate: {e}");
            std::process::exit(3);
        }
    };
    let Some(f) = cand_lib.sym(k.symbol()) else {
        eprintln!("msx check-child: candidate does not export {}", k.symbol());
        std::process::exit(3);
    };

    let len = HEADER + SLOT * 2 * cases as usize;
    let file = std::fs::OpenOptions::new().read(true).write(true).open(&shm_path).map_err(|e| format!("{}: {e}", shm_path.display()))?;
    let base = unsafe {
        use std::os::fd::AsRawFd;
        libc::mmap(core::ptr::null_mut(), len, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, file.as_raw_fd(), 0)
    };
    if base == libc::MAP_FAILED {
        return Err("mmap of the result file failed".into());
    }
    let base = base as *mut u8;
    let progress = base as *mut u64;
    let finished = unsafe { base.add(8) } as *mut u64;

    let mut bufs = Bufs::new();
    let mut done = 0u64;
    for_batch(k, reference, split_of(phase), seed, n, batch, &mut |idx, input, aux| {
        for placement in 0..2usize {
            let run = idx * 2 + placement;
            let p = bufs.place(input, placement == 0);
            unsafe {
                progress.write_volatile(run as u64);
                let res = k.run_case(f, p, input.len(), aux);
                let slot = base.add(HEADER + SLOT * run);
                let (rec, abi) = match res {
                    Ok(r) => (r, 0u8),
                    Err(v) => (OutRec::default(), abi_code(v)),
                };
                core::ptr::copy_nonoverlapping(rec_to_bytes(&rec).as_ptr(), slot, 40);
                slot.add(40).write_volatile(abi);
            }
        }
        done += 1;
    })?;
    unsafe { finished.write_volatile(done) };
    std::process::exit(0);
}

// ---- parent ----

fn timeout_secs() -> u64 {
    std::env::var("MSX_CHECK_TIMEOUT_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(CHECK_TIMEOUT_SECS)
}

/// The `msx` binary used for `__check-child`: `MSX_BIN`, else the running binary if it is `msx`,
/// else the `msx` next to the running binary or one directory up (cargo test layout).
fn msx_binary() -> Result<PathBuf, String> {
    if let Some(p) = std::env::var_os("MSX_BIN") {
        return Ok(PathBuf::from(p));
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    if exe.file_name().is_some_and(|n| n == "msx") {
        return Ok(exe);
    }
    let dir = exe.parent().ok_or("no exe dir")?;
    for cand in [dir.join("msx"), dir.join("..").join("msx")] {
        if cand.is_file() {
            return Ok(cand);
        }
    }
    Err("cannot find the msx binary; set MSX_BIN".into())
}

enum ChildEnd {
    Clean,
    ExitCode(i32),
    Signal(i32),
    TimedOut,
}

fn run_child(cmd: &mut Command, timeout: Duration) -> Result<ChildEnd, String> {
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::null()).spawn().map_err(|e| format!("cannot start the check child: {e}"))?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(st) => {
                return Ok(if let Some(sig) = st.signal() {
                    if sig == libc::SIGXCPU {
                        ChildEnd::TimedOut
                    } else {
                        ChildEnd::Signal(sig)
                    }
                } else if st.code() == Some(0) {
                    ChildEnd::Clean
                } else {
                    ChildEnd::ExitCode(st.code().unwrap_or(-1))
                });
            }
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Ok(ChildEnd::TimedOut);
            }
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    }
}

struct Best {
    len: usize,
    failure: Failure,
}

fn consider(best: &mut Option<Best>, input: &[u8], f: Failure) {
    if best.as_ref().is_none_or(|b| input.len() < b.len) {
        *best = Some(Best { len: input.len(), failure: f });
    }
}

/// Check a candidate; panics with the tool error if the checker itself cannot run.
pub fn check(kernel_name: &str, so: &Path, phase: Phase, seed_file: Option<&Path>) -> CheckReport {
    try_check(kernel_name, so, phase, seed_file).unwrap_or_else(|e| panic!("check failed to run: {e}"))
}

pub fn try_check(kernel_name: &str, so: &Path, phase: Phase, seed_file: Option<&Path>) -> Result<CheckReport, String> {
    let k = kernel(kernel_name).ok_or(format!("unknown kernel {kernel_name}"))?;
    let (seed, n) = phase_params(phase, seed_file)?;
    let split = split_of(phase);
    let ref_path = a_path(Rung::R1);
    if !ref_path.is_file() {
        build_a(Rung::R1).map_err(|e| e.to_string())?;
    }
    let ref_lib = Lib::open(&ref_path)?;
    let reference = ref_lib.sym(k.symbol()).ok_or("A.r1 lacks the kernel symbol")?;
    let total = count_cases(k, reference, split, seed, n)?;
    let batches = total.div_ceil(BATCH_CASES);
    let msx = msx_binary()?;
    let tmpdir = work().join("tmp");
    std::fs::create_dir_all(&tmpdir).map_err(|e| e.to_string())?;
    let timeout = Duration::from_secs(timeout_secs());
    let so_abs = std::fs::canonicalize(so).map_err(|e| format!("{}: {e}", so.display()))?;

    let mut runs_done = 0u64;
    let mut bufs = Bufs::new();
    for batch in 0..batches {
        let cases = (total - batch * BATCH_CASES).min(BATCH_CASES);
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let shm = tmpdir.join(format!("check-{}-{seq}-{batch}.shm", std::process::id()));
        {
            let f = std::fs::File::create(&shm).map_err(|e| e.to_string())?;
            f.set_len((HEADER + SLOT * 2 * cases as usize) as u64).map_err(|e| e.to_string())?;
        }
        let end = run_child(
            Command::new(&msx)
                .arg("__check-child")
                .args(["--kernel", kernel_name, "--phase", phase_name(phase)])
                .arg("--so")
                .arg(&so_abs)
                .args(["--seed", &seed.to_string(), "--n", &n.to_string(), "--batch", &batch.to_string(), "--cases", &cases.to_string()])
                .arg("--shm")
                .arg(&shm),
            timeout,
        );
        let data = std::fs::read(&shm);
        let _ = std::fs::remove_file(&shm);
        let end = end?;
        let data = data.map_err(|e| e.to_string())?;
        let u64_at = |off: usize| u64::from_le_bytes(data[off..off + 8].try_into().unwrap());
        let progress = u64_at(0);

        // Failure that ends the batch: report the run at the progress counter.
        let early = match end {
            ChildEnd::Clean => None,
            ChildEnd::TimedOut => Some(("TIMEOUT", None)),
            ChildEnd::Signal(s) => Some(("OOB_OR_CRASH", Some(s))),
            ChildEnd::ExitCode(c) => {
                eprintln!("msx check: child exited with code {c}");
                Some(("OOB_OR_CRASH", None))
            }
        };
        if let Some((verdict, signal)) = early {
            let at = if matches!(end, ChildEnd::ExitCode(_)) { 0 } else { progress as usize };
            let mut failure = None;
            for_batch(k, reference, split, seed, n, batch, &mut |idx, input, aux| {
                if idx == at / 2 {
                    let p = bufs.place(input, true);
                    let expected = unsafe { k.run_case(reference, p, input.len(), aux) }.unwrap_or_default();
                    failure = Some(Failure {
                        kind: verdict.into(),
                        input_hex: hex(input),
                        aux,
                        placement: if at % 2 == 0 { "end" } else { "start" }.into(),
                        expected,
                        got: None,
                        signal,
                        abi: None,
                    });
                }
            })?;
            return Ok(CheckReport {
                verdict: verdict.into(),
                cases_run: runs_done + progress,
                first_failure: failure,
                mismatches_in_batch: 0,
            });
        }

        if u64_at(8) != cases {
            return Err(format!("child exited cleanly but finished {} of {cases} cases", u64_at(8)));
        }
        // Clean exit: reference answers and comparison.
        let (mut best_abi, mut best_wrong): (Option<Best>, Option<Best>) = (None, None);
        let (mut abi_n, mut wrong_n) = (0u64, 0u64);
        for_batch(k, reference, split, seed, n, batch, &mut |idx, input, aux| {
            let p = bufs.place(input, true);
            let expected = unsafe { k.run_case(reference, p, input.len(), aux) }.expect("reference A.r1 violated the ABI");
            for placement in 0..2usize {
                let off = HEADER + SLOT * (idx * 2 + placement);
                let code = data[off + 40];
                let name = if placement == 0 { "end" } else { "start" };
                if code != 0 {
                    abi_n += 1;
                    consider(
                        &mut best_abi,
                        input,
                        Failure {
                            kind: "ABI_VIOLATION".into(),
                            input_hex: hex(input),
                            aux,
                            placement: name.into(),
                            expected,
                            got: None,
                            signal: None,
                            abi: abi_from_code(code),
                        },
                    );
                } else {
                    let got = rec_from_bytes(&data[off..off + 40]);
                    if got != expected {
                        wrong_n += 1;
                        consider(
                            &mut best_wrong,
                            input,
                            Failure {
                                kind: "INCORRECT".into(),
                                input_hex: hex(input),
                                aux,
                                placement: name.into(),
                                expected,
                                got: Some(got),
                                signal: None,
                                abi: None,
                            },
                        );
                    }
                }
            }
        })?;
        runs_done += cases * 2;
        if let Some(b) = best_abi {
            return Ok(CheckReport {
                verdict: "ABI_VIOLATION".into(),
                cases_run: runs_done,
                first_failure: Some(b.failure),
                mismatches_in_batch: abi_n + wrong_n,
            });
        }
        if let Some(b) = best_wrong {
            return Ok(CheckReport {
                verdict: "INCORRECT".into(),
                cases_run: runs_done,
                first_failure: Some(b.failure),
                mismatches_in_batch: wrong_n,
            });
        }
        eprintln!("msx check: batch {}/{batches} clean ({runs_done} runs)", batch + 1);
    }
    Ok(CheckReport { verdict: "PASS".into(), cases_run: runs_done, first_failure: None, mismatches_in_batch: 0 })
}
