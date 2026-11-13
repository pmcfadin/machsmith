//! One session (spec 6.2, 6.4): both arms on one pinned core, in a seeded order, each a fresh
//! process with its own `CRITERION_HOME` and a seeded environment pad, under `perf stat`.

use crate::criterion::{read_home, CaseStat};
use crate::host::{Probe, Rejection, SessionMonitor};
use rand_chacha::ChaCha8Rng;
use rand_core::{RngCore, SeedableRng};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const SESSION_SEED_BASE: u64 = 0x6D7378;
pub const MAX_PAD: usize = 4095;
pub const SESSION_TIMEOUT: Duration = Duration::from_secs(1800);
pub const CRITERION_ARGS: [&str; 8] =
    ["--bench", "--warm-up-time", "1", "--measurement-time", "3", "--sample-size", "50", "--noplot"];
pub const PERF_EVENTS: &str = "cycles:u,instructions:u";
const STDERR_TAIL: usize = 2048;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Arm {
    pub name: String,
    pub bin: PathBuf,
    pub sha256: String,
}

pub fn sha256_file(p: &Path) -> Result<String, String> {
    let bytes = std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect())
}

/// Round 1's `fresh_nonce`: getrandom, else the clock.
pub fn fresh_nonce() -> u64 {
    let mut b = [0u8; 8];
    let n = unsafe { libc::getrandom(b.as_mut_ptr() as *mut libc::c_void, 8, 0) };
    if n == 8 {
        return u64::from_le_bytes(b);
    }
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
    t ^ ((std::process::id() as u64) << 32)
}

/// Round 1's formula: seed of session `si` after `reruns` rejected sessions.
pub fn session_seed(si: u32, reruns: u32, nonce: u64) -> u64 {
    (SESSION_SEED_BASE + si as u64 + 1000 * reruns as u64) ^ nonce
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct CaseRatio {
    pub base_ns: f64,
    pub cand_ns: f64,
    /// cand median / base median
    pub ratio: f64,
    pub base_mean_ns: f64,
    pub base_mad_ns: f64,
    pub cand_mean_ns: f64,
    pub cand_mad_ns: f64,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct ProcStat {
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub wall_s: f64,
    /// `ru_nivcsw` from wait4 (perf stat plus the bench it waited for)
    pub nivcsw: u64,
    /// what the noise gate judged: `probe.ctx_switches(nivcsw)` (equal unless the probe is fake)
    pub nivcsw_gated: u64,
    pub pad_len: usize,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct SessionRecord {
    /// 0-based count of sessions started in this run (names the scratch dir `s<attempt>`)
    pub attempt: u32,
    /// the valid-session slot this attempt tried to fill
    pub index: u32,
    /// rejections before this attempt
    pub replacements: u32,
    /// hex, so JSON readers keep all 64 bits
    pub seed: String,
    pub core: usize,
    /// roles in run order, e.g. ["cand", "base"]
    pub order: Vec<String>,
    pub cases: BTreeMap<String, CaseRatio>,
    pub perf: BTreeMap<String, Value>,
    pub procs: BTreeMap<String, ProcStat>,
    pub rejection: Option<Rejection>,
}

pub struct SessionInput<'a> {
    pub attempt: u32,
    pub index: u32,
    pub replacements: u32,
    pub seed: u64,
    pub core: usize,
    pub base: &'a Arm,
    pub cand: &'a Arm,
    /// `<scratch>/s<attempt>`
    pub dir: PathBuf,
    /// case ids both arms must report (targets, then the first valid session's cases)
    pub expected: &'a BTreeSet<String>,
    pub probe: Arc<dyn Probe + Send + Sync>,
    pub abort: &'a AtomicBool,
    pub timeout: Duration,
}

pub enum SessionOutcome {
    Done(SessionRecord),
    /// the abort flag was raised; the running arm was killed and nothing is reported
    Aborted,
}

struct ArmRun {
    stat: ProcStat,
    pid: u32,
    failure: Option<String>,
    aborted: bool,
}

fn tail(p: &Path) -> String {
    let b = std::fs::read(p).unwrap_or_default();
    let start = b.len().saturating_sub(STDERR_TAIL);
    String::from_utf8_lossy(&b[start..]).trim().to_string()
}

/// `perf stat -x,` output -> {"cycles:u": n | null, "instructions:u": n | null}; null if absent.
pub fn parse_perf(p: &Path) -> Value {
    let Ok(text) = std::fs::read_to_string(p) else { return Value::Null };
    let mut m = serde_json::Map::new();
    for line in text.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
        let f: Vec<&str> = line.split(',').collect();
        if f.len() >= 3 {
            m.insert(f[2].to_string(), f[0].parse::<u64>().map_or(Value::Null, |v| json!(v)));
        }
    }
    Value::Object(m)
}

fn wait4(pid: i32, flags: i32) -> Result<Option<(i32, libc::rusage)>, String> {
    loop {
        let mut status = 0;
        let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
        let r = unsafe { libc::wait4(pid, &mut status, flags, &mut ru) };
        if r == pid {
            return Ok(Some((status, ru)));
        }
        if r == 0 {
            return Ok(None);
        }
        let e = std::io::Error::last_os_error();
        if e.kind() != std::io::ErrorKind::Interrupted {
            return Err(format!("wait4 {pid}: {e}"));
        }
    }
}

fn run_arm(role: &str, arm: &Arm, inp: &SessionInput, pad_len: usize, deadline: Instant) -> Result<ArmRun, String> {
    let home = inp.dir.join(role);
    std::fs::create_dir_all(&home).map_err(|e| format!("{}: {e}", home.display()))?;
    let out = File::create(inp.dir.join(format!("{role}.stdout"))).map_err(|e| e.to_string())?;
    let errp = inp.dir.join(format!("{role}.stderr"));
    let err = File::create(&errp).map_err(|e| e.to_string())?;
    let mut cmd = Command::new("taskset");
    cmd.arg("-c")
        .arg(inp.core.to_string())
        .args(["perf", "stat", "-x,", "-e", PERF_EVENTS, "-o"])
        .arg(inp.dir.join(format!("{role}.perf")))
        .arg("--")
        .arg(&arm.bin)
        .args(CRITERION_ARGS)
        .env("CRITERION_HOME", &home)
        .env("R2_PAD", "x".repeat(pad_len))
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err)
        .process_group(0); // so an abort or timeout can kill perf and the bench together
    let child = cmd.spawn().map_err(|e| format!("spawn taskset/perf for {role}: {e}"))?;
    let pid = child.id();
    let t0 = Instant::now();
    let mut killed: Option<&str> = None;
    let (status, ru) = loop {
        if let Some(r) = wait4(pid as i32, libc::WNOHANG)? {
            break r;
        }
        if killed.is_none() {
            if inp.abort.load(Ordering::SeqCst) {
                killed = Some("abort");
            } else if Instant::now() >= deadline {
                killed = Some("timeout");
            }
            if killed.is_some() {
                unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    drop(child); // already reaped by wait4; dropping does not wait
    let wall_s = t0.elapsed().as_secs_f64();
    let (exit_code, signal) = if libc::WIFEXITED(status) {
        (Some(libc::WEXITSTATUS(status)), None)
    } else if libc::WIFSIGNALED(status) {
        (None, Some(libc::WTERMSIG(status)))
    } else {
        (None, None)
    };
    let nivcsw = ru.ru_nivcsw.max(0) as u64;
    let stat = ProcStat { exit_code, signal, wall_s, nivcsw, nivcsw_gated: inp.probe.ctx_switches(nivcsw), pad_len };
    let failure = match (killed, exit_code, signal) {
        (Some("abort"), _, _) => None,
        (Some(_), _, _) => Some(format!("{role}: session timeout after {} s", inp.timeout.as_secs())),
        (None, Some(0), _) => None,
        (None, Some(c), _) => Some(format!("{role} exited {c}: {}", tail(&errp))),
        (None, _, Some(s)) => Some(format!("{role} killed by signal {s}: {}", tail(&errp))),
        (None, None, None) => Some(format!("{role}: unknown wait status {status}")),
    };
    Ok(ArmRun { stat, pid, failure, aborted: killed == Some("abort") })
}

fn ratio(b: &CaseStat, c: &CaseStat) -> CaseRatio {
    CaseRatio {
        base_ns: b.median_ns,
        cand_ns: c.median_ns,
        ratio: c.median_ns / b.median_ns,
        base_mean_ns: b.mean_ns,
        base_mad_ns: b.mad_ns,
        cand_mean_ns: c.mean_ns,
        cand_mad_ns: c.mad_ns,
    }
}

/// Runs one session. `Err` is a tool failure (cannot create files or spawn); a misbehaving
/// bench or a noisy host is a `rejection` in the record. Rejection precedence: process failure,
/// then missing case, then the host monitor.
pub fn run_session(inp: &SessionInput) -> Result<SessionOutcome, String> {
    std::fs::create_dir_all(&inp.dir).map_err(|e| format!("{}: {e}", inp.dir.display()))?;
    let mut prng = ChaCha8Rng::seed_from_u64(inp.seed ^ 0x5041_4421);
    let roles: [(&str, &Arm); 2] = if prng.next_u64() & 1 == 1 {
        [("cand", inp.cand), ("base", inp.base)]
    } else {
        [("base", inp.base), ("cand", inp.cand)]
    };
    let pads = [(prng.next_u64() % (MAX_PAD as u64 + 1)) as usize, (prng.next_u64() % (MAX_PAD as u64 + 1)) as usize];
    let deadline = Instant::now() + inp.timeout;
    let mut mon = SessionMonitor::start(inp.probe.clone(), inp.core);
    let mut procs = BTreeMap::new();
    let mut perf = BTreeMap::new();
    let mut failure = None;
    for (i, (role, arm)) in roles.iter().enumerate() {
        let r = run_arm(role, arm, inp, pads[i], deadline)?;
        if r.aborted {
            return Ok(SessionOutcome::Aborted);
        }
        mon.add_process(r.pid, r.stat.nivcsw_gated, r.stat.wall_s);
        procs.insert(role.to_string(), r.stat);
        perf.insert(role.to_string(), parse_perf(&inp.dir.join(format!("{role}.perf"))));
        if r.failure.is_some() {
            failure = r.failure;
            break;
        }
    }
    let monitor = mon.finish();
    let mut cases = BTreeMap::new();
    let rejection = if let Some(f) = failure {
        Some(Rejection::ProcessFailed(f))
    } else {
        match (read_home(&inp.dir.join("base")), read_home(&inp.dir.join("cand"))) {
            (Err(e), _) => Some(Rejection::ProcessFailed(format!("base: {e}"))),
            (_, Err(e)) => Some(Rejection::ProcessFailed(format!("cand: {e}"))),
            (Ok(b), Ok(c)) => {
                let mut ids: BTreeSet<String> = b.keys().chain(c.keys()).cloned().collect();
                if ids.is_empty() {
                    Some(Rejection::ProcessFailed("no Criterion cases in either arm".into()))
                } else {
                    ids.extend(inp.expected.iter().cloned());
                    let missing = ids.iter().find(|id| !b.contains_key(*id) || !c.contains_key(*id)).cloned();
                    for (id, bs) in &b {
                        if let Some(cs) = c.get(id) {
                            cases.insert(id.clone(), ratio(bs, cs));
                        }
                    }
                    missing.map(Rejection::MissingCase).or(monitor)
                }
            }
        }
    };
    Ok(SessionOutcome::Done(SessionRecord {
        attempt: inp.attempt,
        index: inp.index,
        replacements: inp.replacements,
        seed: format!("{:#018x}", inp.seed),
        core: inp.core,
        order: roles.iter().map(|(r, _)| r.to_string()).collect(),
        cases,
        perf,
        procs,
        rejection,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_matches_round1() {
        assert_eq!(session_seed(0, 0, 0), 0x6D7378);
        assert_eq!(session_seed(3, 2, 0xFF), (0x6D7378 + 3 + 2000) ^ 0xFF);
    }

    #[test]
    fn perf_csv_parses() {
        let d = std::env::temp_dir().join(format!("r2-perf-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("x.perf");
        std::fs::write(&p, "# started on x\n\n534854,,cycles:u,364108,100.00,,\n<not counted>,,instructions:u,0,0.00,,\n").unwrap();
        assert_eq!(parse_perf(&p), json!({"cycles:u": 534854, "instructions:u": null}));
        assert_eq!(parse_perf(&d.join("none")), Value::Null);
    }

    #[test]
    fn sha256_known() {
        let d = std::env::temp_dir().join(format!("r2-sha-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("abc"), "abc").unwrap();
        assert_eq!(sha256_file(&d.join("abc")).unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
