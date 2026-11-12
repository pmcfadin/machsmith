//! Measurement: sample hygiene, the per-session child (`msx __session`) and the parent run
//! procedure (`run_protocol`). Protocol constants live here.

use crate::build::{a_path, Rung};
use crate::counters::Counters;
use crate::loader::Lib;
use crate::spec::{kernel, load_segment, manifest_segments, traces_dir, AlignedBuf, KernelSpec, RawFn};
use crate::stats::{bootstrap_ci, median, session_ratio, Interval};
use rand_chacha::ChaCha8Rng;
use rand_core::{RngCore, SeedableRng};
use std::collections::BTreeMap;
use std::path::PathBuf;

// ---- protocol constants (plan README, "Protocol constants") ----
pub const SAMPLE_MIN_OPS: u64 = 50_000;
pub const SMALL_TRACE_MIN_CHUNKS: usize = 20;
pub const SEARCH_SESSIONS: u32 = 6;
pub const SEARCH_BLOCKS: u32 = 60;
pub const FINAL_SESSIONS: u32 = 30;
pub const FINAL_BLOCKS: u32 = 200;
pub const MAX_DISCARD_FRACTION: f64 = 0.30;
pub const MAX_SESSION_RERUNS: u32 = 3;
pub const INSTR_TOLERANCE: f64 = 0.0001;
pub const BOOTSTRAP_RESAMPLES: usize = 10_000;
pub const THRESHOLD_FLOOR: f64 = 0.05;
pub const NULL_BAND_MAX: f64 = 0.05;
// ---- run-procedure constants from the task brief ----
pub const SESSION_SEED_BASE: u64 = 0x6D7378;
pub const MAX_SIBLING_UTIL: f64 = 0.20;
pub const FIXED_CORE: usize = 5;
pub const SIBLING_OFFSET: usize = 16;
pub const WARMUP_RUNS: usize = 3;
pub const MAX_PAD: usize = 4095;
pub const MAX_GAP_PAGES: u64 = 255;
/// Wall-clock limit for one measurement session child.
pub const SESSION_TIMEOUT_SECS: u64 = 1800;
/// Exit code of a session child whose contestant diverged from the reference.
pub const DIVERGED_EXIT: i32 = 4;

/// A contestant that diverged from the reference, died by a signal or hung during measurement.
/// Travels through `run_protocol`'s `Err(String)` as `CONTESTANT_FAILED|name|VERDICT|detail`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContestantFailure {
    pub contestant: String,
    /// INCORRECT (divergence) | OOB_OR_CRASH (signal) | TIMEOUT
    pub verdict: String,
    pub detail: String,
}

impl ContestantFailure {
    pub fn to_error(&self) -> String {
        format!("CONTESTANT_FAILED|{}|{}|{}", self.contestant, self.verdict, self.detail)
    }

    pub fn parse(e: &str) -> Option<ContestantFailure> {
        let rest = e.strip_prefix("CONTESTANT_FAILED|")?;
        let mut it = rest.splitn(3, '|');
        Some(ContestantFailure { contestant: it.next()?.into(), verdict: it.next()?.into(), detail: it.next()?.into() })
    }
}

/// The contestants that are the agent's: a failure of any other contestant is a tool error.
pub fn is_candidate_name(name: &str) -> bool {
    name == "cand" || name.starts_with("B.")
}

/// A fresh 64-bit value for each `run_protocol` call (getrandom, else the clock).
pub fn fresh_nonce() -> u64 {
    let mut b = [0u8; 8];
    let n = unsafe { libc::getrandom(b.as_mut_ptr() as *mut libc::c_void, 8, 0) };
    if n == 8 {
        return u64::from_le_bytes(b);
    }
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
    t ^ ((std::process::id() as u64) << 32)
}

/// Seed of session `si` after `reruns` rejected sessions, made unpredictable by the run's nonce.
pub fn session_seed(si: u32, reruns: u32, nonce: u64) -> u64 {
    (SESSION_SEED_BASE + si as u64 + 1000 * reruns as u64) ^ nonce
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Contestant {
    pub name: String,
    pub so: PathBuf,
    pub symbol: String,
    /// Call `msx_<k>_chain` directly (A.inline).
    pub is_chain: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum Discard {
    Preempted,
    Multiplexed,
    InstrMismatch,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Sample {
    pub session: u32,
    pub block: u32,
    pub chunk: u32,
    pub contestant: String,
    pub ops: u64,
    pub cycles: u64,
    pub instructions: u64,
    pub branch_misses: u64,
    pub wall_ns: u64,
    pub discard: Option<Discard>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Protocol {
    Search,
    Final,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Split {
    Search,
    Holdout,
}
impl Split {
    pub fn as_str(self) -> &'static str {
        match self {
            Split::Search => "search",
            Split::Holdout => "holdout",
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Hygiene
// ---------------------------------------------------------------------------------------------

/// Sets `InstrMismatch` on samples whose instruction count differs from the contestant's modal
/// count on that chunk (within the session) by more than `INSTR_TOLERANCE`, then discards every
/// sample of any block that contains a discarded sample (with the reason of that block's first
/// discarded sample).
///
/// "Mode": the count with the most samples within tolerance of it (ties: the most frequent exact
/// value, then the smallest). Exact repeats are the common case, but user-only counts can jitter by
/// a few instructions, which must not look like "no mode". When no count has any other sample
/// within tolerance, there is no mode and every sample of the group is marked.
pub fn apply_hygiene(samples: &mut [Sample]) {
    let mut groups: BTreeMap<(u32, &str, u32), Vec<usize>> = BTreeMap::new();
    for (i, s) in samples.iter().enumerate() {
        groups.entry((s.session, s.contestant.as_str(), s.chunk)).or_default().push(i);
    }
    let mut mismatched = Vec::new();
    for idx in groups.values() {
        let vals: Vec<u64> = idx.iter().map(|&i| samples[i].instructions).collect();
        let within = |a: u64, b: u64| (a as f64 - b as f64).abs() <= INSTR_TOLERANCE * (b as f64);
        // (support, exact count, value)
        let mut best: Option<(usize, usize, u64)> = None;
        for &v in &vals {
            let support = vals.iter().filter(|&&x| within(x, v)).count();
            let exact = vals.iter().filter(|&&x| x == v).count();
            let better = match best {
                None => true,
                Some((bs, be, bv)) => (support, exact, std::cmp::Reverse(v)) > (bs, be, std::cmp::Reverse(bv)),
            };
            if better {
                best = Some((support, exact, v));
            }
        }
        let (support, _, mode) = best.expect("non-empty group");
        let no_mode = vals.len() >= 2 && support < 2;
        for (&i, &v) in idx.iter().zip(&vals) {
            if no_mode || !within(v, mode) {
                mismatched.push(i);
            }
        }
    }
    for i in mismatched {
        if samples[i].discard.is_none() {
            samples[i].discard = Some(Discard::InstrMismatch);
        }
    }
    let mut cause: BTreeMap<(u32, u32), Discard> = BTreeMap::new();
    for s in samples.iter() {
        if let Some(d) = s.discard {
            cause.entry((s.session, s.block)).or_insert(d);
        }
    }
    for s in samples.iter_mut() {
        if s.discard.is_none() {
            s.discard = cause.get(&(s.session, s.block)).copied();
        }
    }
}

fn block_counts(samples: &[Sample]) -> (usize, usize) {
    let mut blocks: BTreeMap<(u32, u32), bool> = BTreeMap::new();
    for s in samples {
        *blocks.entry((s.session, s.block)).or_insert(false) |= s.discard.is_some();
    }
    (blocks.values().filter(|&&d| d).count(), blocks.len())
}

/// Discarded blocks <= `MAX_DISCARD_FRACTION` of the session's blocks.
pub fn session_is_valid(samples: &[Sample]) -> bool {
    let (bad, total) = block_counts(samples);
    total > 0 && (bad as f64) / (total as f64) <= MAX_DISCARD_FRACTION
}

// ---------------------------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------------------------

fn shuffle<T>(v: &mut [T], rng: &mut ChaCha8Rng) {
    for i in (1..v.len()).rev() {
        let j = (rng.next_u64() % (i as u64 + 1)) as usize;
        v.swap(i, j);
    }
}

fn now_raw_ns() -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC_RAW, &mut ts) };
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

fn involuntary_switches() -> u64 {
    let mut ru: libc::rusage = unsafe { core::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_THREAD, &mut ru) };
    ru.ru_nivcsw as u64
}

fn cpu_freq_khz(core: usize) -> Option<u64> {
    std::fs::read_to_string(format!("/sys/devices/system/cpu/cpu{core}/cpufreq/scaling_cur_freq")).ok()?.trim().parse().ok()
}

/// Plain summing loop that touches every byte; the result is kept alive.
#[inline(never)]
fn touch(parts: &[&[u8]]) -> u64 {
    let mut acc = 0u64;
    for p in parts {
        for &b in p.iter() {
            acc = acc.wrapping_add(b as u64);
        }
    }
    std::hint::black_box(acc)
}

fn arg_after(args: &[String], flag: &str) -> Result<String, String> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned().ok_or(format!("{flag} required"))
}

// ---------------------------------------------------------------------------------------------
// Child: msx __session
// ---------------------------------------------------------------------------------------------

/// Loads `so` from a private copy (new inode, unique path) so that dlopen can never hand back a
/// handle that is already mapped (for example the A.r1 reference). The copy is unlinked once loaded.
pub fn load_private_copy(so: &std::path::Path, tag: &str) -> Result<Lib, String> {
    let dir = crate::build::work().join("tmp").join(format!("sess-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let copy = dir.join(format!("{tag}.so"));
    std::fs::copy(so, &copy).map_err(|e| format!("copy {}: {e}", so.display()))?;
    let r = Lib::open(&copy);
    let _ = std::fs::remove_file(&copy);
    let _ = std::fs::remove_dir(&dir);
    r
}

/// A session is only usable if every requested comparison (ordered pair of contestants) has a
/// ratio, i.e. at least 10 valid blocks. Returns the first missing pair.
pub fn missing_ratio(samples: &[Sample], names: &[&str]) -> Option<(String, String)> {
    for c in names {
        for b in names {
            if c != b && session_ratio(samples, c, b).is_none() {
                return Some((c.to_string(), b.to_string()));
            }
        }
    }
    None
}

type ChainFn = unsafe extern "C" fn(*const u8, usize, *mut u64) -> u64;

enum Runner {
    Kernel(RawFn),
    Chain(ChainFn),
}

impl Runner {
    #[inline(always)]
    unsafe fn run(&self, k: &dyn KernelSpec, chunk: &[u8]) -> (u64, u64) {
        match self {
            Runner::Kernel(f) => k.run_chain(*f, chunk),
            Runner::Chain(c) => {
                let mut sum = 0u64;
                let ops = c(chunk.as_ptr(), chunk.len(), &mut sum);
                (ops, sum)
            }
        }
    }
}

/// Runs every part in order and sums operations and checksums.
#[inline(always)]
unsafe fn run_parts(r: &Runner, k: &dyn KernelSpec, parts: &[&[u8]]) -> (u64, u64) {
    let (mut ops, mut sum) = (0u64, 0u64);
    for p in parts {
        let (o, s) = r.run(k, p);
        ops += o;
        sum = sum.wrapping_add(s);
    }
    (ops, sum)
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ChildMeta {
    pub load_order: Vec<String>,
    pub gap_pages: Vec<u64>,
    pub freq_start_khz: Option<u64>,
    pub freq_end_khz: Option<u64>,
    pub env_pad_len: usize,
    pub inputs_repeat: bool,
    pub repeat_r: Option<u64>,
    pub chunks_available: usize,
    pub chunks_used: usize,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ChildOutput {
    samples: Vec<Sample>,
    meta: ChildMeta,
}

fn small_trace_flag(kernel: &str, split: Split) -> Result<bool, String> {
    let text = std::fs::read_to_string(traces_dir().join(kernel).join("MANIFEST.json")).map_err(|e| format!("{kernel}: {e}"))?;
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("{kernel}: {e}"))?;
    Ok(v["small_trace"][split.as_str()].as_bool().unwrap_or(false))
}

fn pin_to_core(core: usize) -> Result<(), String> {
    unsafe {
        let mut set: libc::cpu_set_t = core::mem::zeroed();
        libc::CPU_SET(core, &mut set);
        if libc::sched_setaffinity(0, core::mem::size_of::<libc::cpu_set_t>(), &set) != 0 {
            return Err(format!("sched_setaffinity({core}): {}", std::io::Error::last_os_error()));
        }
    }
    Ok(())
}

/// A shared 8-byte mark that the child sets to the index of the contestant it is about to run, so
/// the parent can say who was running when the child died or hung. `u64::MAX` = none.
struct Progress(*mut u64);

impl Progress {
    fn open(path: &str) -> Result<Progress, String> {
        use std::os::fd::AsRawFd;
        let f = std::fs::OpenOptions::new().read(true).write(true).open(path).map_err(|e| format!("{path}: {e}"))?;
        let p = unsafe { libc::mmap(core::ptr::null_mut(), 8, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, f.as_raw_fd(), 0) };
        if p == libc::MAP_FAILED {
            return Err(format!("mmap of {path} failed"));
        }
        Ok(Progress(p as *mut u64))
    }
    fn set(&self, i: u64) {
        unsafe { self.0.write_volatile(i) };
    }
}

/// `msx __session --kernel K --split S --session I --seed N --core C --blocks B --contestants JSON
///  [--progress FILE]`
/// Prints `{samples, meta}` as JSON. Divergence from the reference exits with `DIVERGED_EXIT`.
pub fn session_main(args: &[String]) -> Result<(), String> {
    let kname = arg_after(args, "--kernel")?;
    let k = kernel(&kname).ok_or(format!("unknown kernel {kname}"))?;
    let split = match arg_after(args, "--split")?.as_str() {
        "search" => Split::Search,
        "holdout" => Split::Holdout,
        o => return Err(format!("bad --split {o}")),
    };
    let session: u32 = arg_after(args, "--session")?.parse().map_err(|e| format!("--session: {e}"))?;
    let seed: u64 = arg_after(args, "--seed")?.parse().map_err(|e| format!("--seed: {e}"))?;
    let core: usize = arg_after(args, "--core")?.parse().map_err(|e| format!("--core: {e}"))?;
    let blocks: u32 = arg_after(args, "--blocks")?.parse().map_err(|e| format!("--blocks: {e}"))?;
    let progress = match arg_after(args, "--progress") {
        Ok(p) => Some(Progress::open(&p)?),
        Err(_) => None,
    };
    let mark = |i: u64| {
        if let Some(p) = &progress {
            p.set(i);
        }
    };
    let contestants: Vec<Contestant> = serde_json::from_str(&arg_after(args, "--contestants")?).map_err(|e| format!("--contestants: {e}"))?;
    if contestants.is_empty() || blocks == 0 {
        return Err("need at least one contestant and one block".into());
    }

    // 1. pin
    pin_to_core(core)?;
    let counters = Counters::open()?;

    mark(u64::MAX);
    // 2. reference first (never unloaded), then contestants in a seeded order with random gaps
    let ref_lib = Lib::open(&a_path(Rung::R1))?;
    let reference = ref_lib.sym(k.symbol()).ok_or("A.r1 lacks the kernel symbol")?;
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut load_idx: Vec<usize> = (0..contestants.len()).collect();
    shuffle(&mut load_idx, &mut rng);
    let mut libs: Vec<Option<Lib>> = (0..contestants.len()).map(|_| None).collect();
    let mut runners: Vec<Option<Runner>> = (0..contestants.len()).map(|_| None).collect();
    let mut gaps: Vec<(*mut libc::c_void, usize)> = Vec::new();
    let mut gap_pages = Vec::new();
    let mut load_order = Vec::new();
    for &i in &load_idx {
        let pages = rng.next_u64() % (MAX_GAP_PAGES + 1);
        if pages > 0 {
            let len = pages as usize * 4096;
            let p = unsafe { libc::mmap(core::ptr::null_mut(), len, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_PRIVATE | libc::MAP_ANONYMOUS, -1, 0) };
            if p != libc::MAP_FAILED {
                gaps.push((p, len));
            }
        }
        gap_pages.push(pages);
        let c = &contestants[i];
        let lib = load_private_copy(&c.so, &format!("s{session}-{i}")).map_err(|e| format!("{}: {e}", c.name))?;
        let f = lib.sym(&c.symbol).ok_or(format!("{}: {} not exported by {}", c.name, c.symbol, c.so.display()))?;
        runners[i] = Some(if c.is_chain { Runner::Chain(unsafe { core::mem::transmute::<RawFn, ChainFn>(f) }) } else { Runner::Kernel(f) });
        libs[i] = Some(lib);
        load_order.push(c.name.clone());
    }
    let runners: Vec<Runner> = runners.into_iter().map(|r| r.expect("loaded")).collect();

    // 3. inputs
    let segs = manifest_segments(&kname, Some(split.as_str()))?;
    if segs.is_empty() {
        return Err(format!("{kname}: no {} segments", split.as_str()));
    }
    let bufs: Vec<AlignedBuf> = segs
        .iter()
        .map(|s| load_segment(&s.path).map_err(|e| format!("{}: {e}", s.path.display())))
        .collect::<Result<_, _>>()?;
    let seg_slices: Vec<&[u8]> = bufs.iter().map(|b| b.as_slice()).collect();
    let small = small_trace_flag(&kname, split)?;

    // Per block: the parts handed to run_chain, the bytes to touch, the reference result.
    struct BlockInput<'a> {
        chunk: u32,
        parts: Vec<&'a [u8]>,
        touch: Vec<&'a [u8]>,
        reference: (u64, u64),
    }
    let mut inputs: Vec<BlockInput> = Vec::new();
    let (mut chunks_available, mut chunks_used, mut repeat_r) = (0usize, 0usize, None);
    let warm_parts: Vec<&[u8]>;
    if small {
        let mut pass = (0u64, 0u64);
        for s in &seg_slices {
            let (o, c) = unsafe { k.run_chain(reference, s) };
            pass = (pass.0 + o, pass.1.wrapping_add(c));
        }
        if pass.0 == 0 {
            return Err(format!("{kname}: the {} split has no operations", split.as_str()));
        }
        let r = SAMPLE_MIN_OPS.div_ceil(pass.0);
        repeat_r = Some(r);
        for b in 0..blocks {
            // the same order for every contestant in the block, a fresh shuffle per pass
            let mut brng = ChaCha8Rng::seed_from_u64(seed ^ (0x9E37_79B9_7F4A_7C15u64.wrapping_mul(b as u64 + 1)));
            let mut parts = Vec::with_capacity(r as usize * seg_slices.len());
            for _ in 0..r {
                let mut order: Vec<usize> = (0..seg_slices.len()).collect();
                shuffle(&mut order, &mut brng);
                parts.extend(order.into_iter().map(|i| seg_slices[i]));
            }
            inputs.push(BlockInput { chunk: 0, parts, touch: seg_slices.clone(), reference: (pass.0 * r, pass.1.wrapping_mul(r)) });
        }
        warm_parts = seg_slices.clone();
    } else {
        let mut all: Vec<&[u8]> = Vec::new();
        for s in &seg_slices {
            all.extend(unsafe { k.chunks(reference, s, SAMPLE_MIN_OPS) });
        }
        chunks_available = all.len();
        if all.len() < SMALL_TRACE_MIN_CHUNKS {
            eprintln!("msx session: warning: only {} chunks although the manifest does not mark {} small-trace", all.len(), split.as_str());
        }
        if all.is_empty() {
            return Err(format!("{kname}: the {} split yields no chunk of {SAMPLE_MIN_OPS} operations", split.as_str()));
        }
        let d = all.len().min((blocks as usize / 3).max(1));
        chunks_used = d;
        let mut order: Vec<usize> = (0..all.len()).collect();
        shuffle(&mut order, &mut rng);
        let chosen: Vec<&[u8]> = order[..d].iter().map(|&i| all[i]).collect();
        let refs: Vec<(u64, u64)> = chosen.iter().map(|c| unsafe { k.run_chain(reference, c) }).collect();
        for b in 0..blocks as usize {
            let ci = b % d;
            inputs.push(BlockInput { chunk: ci as u32, parts: vec![chosen[ci]], touch: vec![chosen[ci]], reference: refs[ci] });
        }
        warm_parts = vec![chosen[0]];
    }

    // 5. warm-up (unmeasured), every contestant
    for (ci, r) in runners.iter().enumerate() {
        mark(ci as u64);
        for _ in 0..WARMUP_RUNS {
            std::hint::black_box(unsafe { run_parts(r, k, &warm_parts) });
        }
    }

    // 6. blocks
    let freq_start_khz = cpu_freq_khz(core);
    let mut samples = Vec::with_capacity(blocks as usize * contestants.len());
    for (b, inp) in inputs.iter().enumerate() {
        let mut order: Vec<usize> = (0..contestants.len()).collect();
        shuffle(&mut order, &mut rng);
        for &ci in &order {
            mark(ci as u64);
            touch(&inp.touch);
            let inv0 = involuntary_switches();
            let t0 = now_raw_ns();
            let r0 = counters.read();
            let (ops, sum) = unsafe { run_parts(&runners[ci], k, &inp.parts) };
            let r1 = counters.read();
            let t1 = now_raw_ns();
            let inv1 = involuntary_switches();
            if (ops, sum) != inp.reference {
                // The parent attributes this through the progress mark and the exit code.
                eprintln!(
                    "contestant {} diverged during measurement (block {b}): got ops={ops} checksum={sum:#x}, reference ops={} checksum={:#x}",
                    contestants[ci].name, inp.reference.0, inp.reference.1
                );
                std::process::exit(DIVERGED_EXIT);
            }
            let discard = if inv1 > inv0 {
                Some(Discard::Preempted)
            } else if r1.time_enabled - r0.time_enabled != r1.time_running - r0.time_running {
                Some(Discard::Multiplexed)
            } else {
                None
            };
            samples.push(Sample {
                session,
                block: b as u32,
                chunk: inp.chunk,
                contestant: contestants[ci].name.clone(),
                ops,
                cycles: r1.cycles - r0.cycles,
                instructions: r1.instructions - r0.instructions,
                branch_misses: r1.branch_misses - r0.branch_misses,
                wall_ns: t1 - t0,
                discard,
            });
        }
    }
    let freq_end_khz = cpu_freq_khz(core);

    // 7. output
    let meta = ChildMeta {
        load_order,
        gap_pages,
        freq_start_khz,
        freq_end_khz,
        env_pad_len: std::env::var("MSX_PAD").map(|v| v.len()).unwrap_or(0),
        inputs_repeat: small,
        repeat_r,
        chunks_available,
        chunks_used,
    };
    let out = serde_json::to_string(&ChildOutput { samples, meta }).map_err(|e| e.to_string())?;
    println!("{out}");
    drop((libs, gaps));
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Parent: run_protocol
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, serde::Serialize)]
pub struct SessionMeta {
    pub session: u32,
    pub seed: u64,
    pub reruns_before: u32,
    pub core: usize,
    pub sibling_utilization: Option<f64>,
    pub freq_start_khz: Option<u64>,
    pub freq_end_khz: Option<u64>,
    pub discarded_fraction: f64,
    pub total_blocks: u32,
    pub valid_blocks: u32,
    pub env_pad_len: usize,
    pub load_order: Vec<String>,
    pub gap_pages: Vec<u64>,
    pub chunks_available: usize,
    pub chunks_used: usize,
    pub valid: bool,
    pub invalid_reason: Option<String>,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct Medians {
    pub cycles_per_op: f64,
    pub ns_per_op: f64,
    pub instructions_per_op: f64,
    pub branch_misses_per_op: f64,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct PairResult {
    pub cand: String,
    pub base: String,
    pub session_ratios: Vec<f64>,
    pub interval: Option<Interval>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct RunResult {
    pub kernel: String,
    pub protocol: String,
    pub split: String,
    pub sessions_used: u32,
    pub blocks_used: u32,
    /// MSX_SESSIONS / MSX_BLOCKS overrode the protocol sizes (never ledger-eligible).
    pub size_override: bool,
    pub core: usize,
    pub core_choice: String,
    pub randomize_va_space: Option<u32>,
    pub warnings: Vec<String>,
    pub inputs_repeat: bool,
    pub repeat_r: Option<u64>,
    pub reruns: u32,
    /// Hex of the per-run value XORed into every session seed (the seeds are not reproducible).
    pub nonce: String,
    pub medians: BTreeMap<String, Medians>,
    pub pairs: Vec<PairResult>,
    /// Accepted sessions.
    pub sessions: Vec<SessionMeta>,
    /// Sessions rejected and rerun.
    pub rejected_sessions: Vec<SessionMeta>,
    #[serde(skip)]
    pub samples: Vec<Sample>,
}

impl RunResult {
    pub fn pair(&self, cand: &str, base: &str) -> Option<&PairResult> {
        self.pairs.iter().find(|p| p.cand == cand && p.base == base)
    }
}

/// (busy, total) jiffies per CPU from /proc/stat.
fn read_proc_stat() -> BTreeMap<usize, (u64, u64)> {
    let mut m = BTreeMap::new();
    let Ok(text) = std::fs::read_to_string("/proc/stat") else { return m };
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let Some(name) = it.next() else { continue };
        let Some(n) = name.strip_prefix("cpu").and_then(|n| n.parse::<usize>().ok()) else { continue };
        let f: Vec<u64> = it.take(8).filter_map(|x| x.parse().ok()).collect();
        if f.len() < 5 {
            continue;
        }
        let total: u64 = f.iter().sum();
        m.insert(n, (total - f[3] - f[4], total));
    }
    m
}

fn util(before: &BTreeMap<usize, (u64, u64)>, after: &BTreeMap<usize, (u64, u64)>, cpu: usize) -> Option<f64> {
    let (b0, t0) = before.get(&cpu)?;
    let (b1, t1) = after.get(&cpu)?;
    let dt = t1.checked_sub(*t0)?;
    if dt == 0 {
        return None;
    }
    Some(b1.saturating_sub(*b0) as f64 / dt as f64)
}

/// Returns (core, "auto" | "fixed").
fn choose_core() -> (usize, &'static str) {
    let before = read_proc_stat();
    std::thread::sleep(std::time::Duration::from_millis(500));
    let after = read_proc_stat();
    let mut best: Option<(f64, usize)> = None;
    for c in 2..=15usize {
        match (util(&before, &after, c), util(&before, &after, c + SIBLING_OFFSET)) {
            (Some(a), Some(b)) => {
                if best.map_or(true, |(u, _)| a + b < u) {
                    best = Some((a + b, c));
                }
            }
            _ => return (FIXED_CORE, "fixed"),
        }
    }
    match best {
        Some((_, c)) => (c, "auto"),
        None => (FIXED_CORE, "fixed"),
    }
}

fn session_exe() -> Result<PathBuf, String> {
    match std::env::var_os("MSX_EXE") {
        Some(p) => Ok(PathBuf::from(p)),
        None => std::env::current_exe().map_err(|e| format!("current_exe: {e}")),
    }
}

fn env_u32(name: &str) -> Result<Option<u32>, String> {
    match std::env::var(name) {
        Ok(v) => v.parse().map(Some).map_err(|e| format!("{name}={v}: {e}")),
        Err(_) => Ok(None),
    }
}

/// True when MSX_SESSIONS or MSX_BLOCKS is set; Task 9 refuses ledger records then.
pub fn size_override_active() -> bool {
    std::env::var_os("MSX_SESSIONS").is_some() || std::env::var_os("MSX_BLOCKS").is_some()
}

pub fn run_protocol(kname: &str, contestants: &[Contestant], protocol: Protocol, split: Split) -> Result<RunResult, String> {
    run_protocol_with(kname, contestants, protocol, split, std::time::Duration::from_secs(SESSION_TIMEOUT_SECS))
}

/// `run_protocol` with an explicit per-session wall-clock limit. A candidate (`cand`, `B.*`) that
/// diverges, dies by a signal or exceeds the limit makes this return a `ContestantFailure` error
/// (see `ContestantFailure::parse`); any other contestant failing is a plain tool error.
pub fn run_protocol_with(
    kname: &str,
    contestants: &[Contestant],
    protocol: Protocol,
    split: Split,
    session_timeout: std::time::Duration,
) -> Result<RunResult, String> {
    let k = kernel(kname).ok_or(format!("unknown kernel {kname}"))?;
    if !crate::spec::kernels_with_traces().iter().any(|t| t.name() == kname) {
        return Err(format!("{kname}: no recorded traces, nothing to measure"));
    }
    if contestants.is_empty() {
        return Err("no contestants".into());
    }
    let mut names: Vec<&str> = contestants.iter().map(|c| c.name.as_str()).collect();
    names.sort();
    if names.windows(2).any(|w| w[0] == w[1]) {
        return Err("contestant names must be unique".into());
    }
    let (def_s, def_b) = match protocol {
        Protocol::Search => (SEARCH_SESSIONS, SEARCH_BLOCKS),
        Protocol::Final => (FINAL_SESSIONS, FINAL_BLOCKS),
    };
    let sessions_n = env_u32("MSX_SESSIONS")?.unwrap_or(def_s);
    let blocks_n = env_u32("MSX_BLOCKS")?.unwrap_or(def_b);
    let size_override = size_override_active();

    // 1. ASLR
    let mut warnings = Vec::new();
    let aslr = std::fs::read_to_string("/proc/sys/kernel/randomize_va_space").ok().and_then(|s| s.trim().parse::<u32>().ok());
    if aslr != Some(2) {
        warnings.push(format!("randomize_va_space is {aslr:?}, expected 2; not changed"));
    }
    // 2. core
    let (core, core_choice) = choose_core();
    eprintln!("msx measure: core {core} ({core_choice}), {sessions_n} sessions x {blocks_n} blocks{}", if size_override { " [size override]" } else { "" });

    let exe = session_exe()?;
    let cjson = serde_json::to_string(contestants).map_err(|e| e.to_string())?;
    let mut reruns = 0u32;
    let mut accepted: Vec<(SessionMeta, Vec<Sample>)> = Vec::new();
    let mut rejected: Vec<SessionMeta> = Vec::new();
    let (mut inputs_repeat, mut repeat_r) = (false, None);
    let nonce = fresh_nonce();
    let tmp = crate::build::work().join("tmp");
    std::fs::create_dir_all(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;

    let mut si = 0u32;
    while si < sessions_n {
        // 3. spawn
        let seed = session_seed(si, reruns, nonce);
        let mut prng = ChaCha8Rng::seed_from_u64(seed ^ 0x5041_4421);
        let pad_len = (prng.next_u64() % (MAX_PAD as u64 + 1)) as usize;
        let pad = "x".repeat(pad_len);
        let sib = core + SIBLING_OFFSET;
        let before = read_proc_stat();
        let prog_path = tmp.join(format!("progress-{}-{si}-{reruns}.bin", std::process::id()));
        std::fs::write(&prog_path, u64::MAX.to_le_bytes()).map_err(|e| format!("{}: {e}", prog_path.display()))?;
        let mut cmd = std::process::Command::new(&exe);
        cmd.arg("__session")
            .args(["--kernel", kname, "--split", split.as_str()])
            .args(["--session", &si.to_string(), "--seed", &seed.to_string()])
            .args(["--core", &core.to_string(), "--blocks", &blocks_n.to_string()])
            .args(["--contestants", &cjson])
            .arg("--progress")
            .arg(&prog_path)
            .env("MSX_PAD", &pad);
        let run = crate::build::run_with_timeout(&mut cmd, session_timeout).map_err(|e| format!("spawn {}: {e}", exe.display()));
        let progress = std::fs::read(&prog_path).ok().and_then(|b| b.get(..8).map(|x| u64::from_le_bytes(x.try_into().unwrap())));
        let _ = std::fs::remove_file(&prog_path);
        let run = run?;
        let after = read_proc_stat();
        let stderr_text = String::from_utf8_lossy(&run.stderr).trim().to_string();
        let running = progress.and_then(|p| contestants.get(p as usize));
        // A candidate that misbehaves is a result; anything else that goes wrong is a tool error.
        let blame = |verdict: &str, detail: String| -> String {
            match running {
                Some(c) if is_candidate_name(&c.name) => ContestantFailure { contestant: c.name.clone(), verdict: verdict.into(), detail }.to_error(),
                Some(c) => format!("session {si}: contestant {} ({verdict}): {detail}", c.name),
                None => format!("session {si} ({verdict}): {detail}"),
            }
        };
        let status = match run.status {
            None => return Err(blame("TIMEOUT", format!("the session exceeded {} s", session_timeout.as_secs()))),
            Some(st) => st,
        };
        if let Some(sig) = std::os::unix::process::ExitStatusExt::signal(&status) {
            return Err(blame("OOB_OR_CRASH", format!("the session died of signal {sig}")));
        }
        if status.code() == Some(DIVERGED_EXIT) {
            return Err(blame("INCORRECT", stderr_text));
        }
        if !status.success() {
            return Err(format!("session {si} failed ({status}): {stderr_text}"));
        }
        let child: ChildOutput = serde_json::from_slice(&run.stdout).map_err(|e| format!("session {si}: bad output: {e}"))?;
        let mut samples = child.samples;
        let cm = child.meta;
        inputs_repeat = cm.inputs_repeat;
        repeat_r = cm.repeat_r;
        // 4. validity
        let sibling_util = util(&before, &after, sib);
        apply_hygiene(&mut samples);
        let (bad, total) = block_counts(&samples);
        let mut meta = SessionMeta {
            session: si,
            seed,
            reruns_before: reruns,
            core,
            sibling_utilization: sibling_util,
            freq_start_khz: cm.freq_start_khz,
            freq_end_khz: cm.freq_end_khz,
            discarded_fraction: if total > 0 { bad as f64 / total as f64 } else { 1.0 },
            total_blocks: total as u32,
            valid_blocks: (total - bad) as u32,
            env_pad_len: cm.env_pad_len,
            load_order: cm.load_order,
            gap_pages: cm.gap_pages,
            chunks_available: cm.chunks_available,
            chunks_used: cm.chunks_used,
            valid: true,
            invalid_reason: None,
        };
        if sibling_util.is_some_and(|u| u > MAX_SIBLING_UTIL) {
            meta.valid = false;
            meta.invalid_reason = Some(format!("sibling cpu{sib} utilization {:.0}% > {:.0}%", sibling_util.unwrap() * 100.0, MAX_SIBLING_UTIL * 100.0));
        } else if !session_is_valid(&samples) {
            meta.valid = false;
            meta.invalid_reason = Some(format!("{bad} of {total} blocks discarded (> {:.0}%)", MAX_DISCARD_FRACTION * 100.0));
        } else if let Some((c, b)) = missing_ratio(&samples, &names) {
            meta.valid = false;
            meta.invalid_reason = Some(format!("fewer than 10 valid blocks for {c}/{b}: no session ratio"));
        }
        if meta.valid {
            eprintln!("msx measure: session {si} ok ({} of {} blocks valid)", meta.valid_blocks, meta.total_blocks);
            accepted.push((meta, samples));
            si += 1;
        } else {
            eprintln!("msx measure: session {si} invalid: {}", meta.invalid_reason.as_deref().unwrap_or(""));
            rejected.push(meta);
            reruns += 1;
            if reruns > MAX_SESSION_RERUNS {
                return Err(format!(
                    "NOISE_GATE_FAILED: {reruns} invalid sessions exceed MAX_SESSION_RERUNS={MAX_SESSION_RERUNS} (core {core}; last: {})",
                    rejected.last().and_then(|m| m.invalid_reason.clone()).unwrap_or_default()
                ));
            }
        }
    }

    // statistics
    let mut medians = BTreeMap::new();
    for c in contestants {
        let (mut cy, mut ns, mut ins, mut br) = (vec![], vec![], vec![], vec![]);
        for (_, samples) in &accepted {
            let (mut a, mut b, mut c2, mut d) = (vec![], vec![], vec![], vec![]);
            for s in samples.iter().filter(|s| s.contestant == c.name && s.discard.is_none() && s.ops > 0) {
                let o = s.ops as f64;
                a.push(s.cycles as f64 / o);
                b.push(s.wall_ns as f64 / o);
                c2.push(s.instructions as f64 / o);
                d.push(s.branch_misses as f64 / o);
            }
            cy.push(median(&mut a));
            ns.push(median(&mut b));
            ins.push(median(&mut c2));
            br.push(median(&mut d));
        }
        medians.insert(
            c.name.clone(),
            Medians { cycles_per_op: median(&mut cy), ns_per_op: median(&mut ns), instructions_per_op: median(&mut ins), branch_misses_per_op: median(&mut br) },
        );
    }
    let mut pairs = Vec::new();
    for cand in contestants {
        for base in contestants {
            if cand.name == base.name {
                continue;
            }
            let ratios: Vec<f64> = accepted.iter().filter_map(|(_, s)| session_ratio(s, &cand.name, &base.name)).collect();
            let interval = if ratios.is_empty() { None } else { Some(bootstrap_ci(&ratios)) };
            pairs.push(PairResult { cand: cand.name.clone(), base: base.name.clone(), session_ratios: ratios, interval });
        }
    }
    let _ = k;
    let mut all_samples = Vec::new();
    let mut sessions = Vec::new();
    for (m, s) in accepted {
        sessions.push(m);
        all_samples.extend(s);
    }
    Ok(RunResult {
        kernel: kname.into(),
        protocol: format!("{protocol:?}").to_lowercase(),
        split: split.as_str().into(),
        sessions_used: sessions_n,
        blocks_used: blocks_n,
        size_override,
        core,
        core_choice: core_choice.into(),
        randomize_va_space: aslr,
        warnings,
        inputs_repeat,
        repeat_r,
        reruns,
        nonce: format!("{nonce:#018x}"),
        medians,
        pairs,
        sessions,
        rejected_sessions: rejected,
        samples: all_samples,
    })
}
