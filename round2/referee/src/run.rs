//! A run (spec 6.2-6.7): lock, refusals, quiet wait, sessions with rejection and replacement,
//! verdicts, and exactly one ledger record whatever happens after the lock is taken.

use crate::host::{choose_core, quiet_wait, snapshot, Probe, QUIET_HOLD, QUIET_MAX_LOAD};
use crate::ledger;
use crate::session::{
    fresh_nonce, run_session, session_seed, sha256_file, Arm, SessionInput, SessionOutcome, SessionRecord, CRITERION_ARGS,
    PERF_EVENTS, SESSION_TIMEOUT,
};
use crate::verdicts::{null_fields, ratio_maps, verdict_fields};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;
use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub use crate::verdicts::null_band;

pub const CONFIRM_SESSIONS: u32 = 20;
pub const SCREEN_SESSIONS: u32 = 5;
pub const CROSSCHECK_SESSIONS: u32 = 10;
pub const NULL_SESSIONS: u32 = 20;
pub const MAX_SESSION_REPLACEMENTS: u32 = 3;
pub const MAX_CONFIRM_TRIES: u32 = 3;
pub const NULL_BAND_MAX: f64 = 0.05;
pub const NULL_MAX_AGE_DAYS: u64 = 7;
const LOCK_GRACE: Duration = Duration::from_secs(5);
pub const QUIET_TIMEOUT: Duration = Duration::from_secs(12 * 3600);

/// Raised by the CLI's SIGINT/SIGTERM handlers; the run kills the running arm and records `aborted`.
pub static ABORT: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunKind {
    Null,
    Calibration,
    Screen,
    Confirm,
    Crosscheck,
    Proposal,
}

impl RunKind {
    pub fn as_str(self) -> &'static str {
        match self {
            RunKind::Null => "null",
            RunKind::Calibration => "calibration",
            RunKind::Screen => "screen",
            RunKind::Confirm => "confirm",
            RunKind::Crosscheck => "crosscheck",
            RunKind::Proposal => "proposal",
        }
    }

    pub fn parse(s: &str) -> Option<RunKind> {
        [RunKind::Null, RunKind::Calibration, RunKind::Screen, RunKind::Confirm, RunKind::Crosscheck, RunKind::Proposal]
            .into_iter()
            .find(|k| k.as_str() == s)
    }

    /// The protocol's session count (spec 6.2, 6.3). Calibration and proposal runs use the
    /// confirmation count.
    pub fn protocol_sessions(self) -> u32 {
        match self {
            RunKind::Null => NULL_SESSIONS,
            RunKind::Screen => SCREEN_SESSIONS,
            RunKind::Crosscheck => CROSSCHECK_SESSIONS,
            RunKind::Calibration | RunKind::Confirm | RunKind::Proposal => CONFIRM_SESSIONS,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RunSpec {
    pub kind: RunKind,
    pub workload: String,
    pub base: Arm,
    pub cand: Arm,
    pub sessions: u32,
    pub targets: Vec<String>,
    pub rule: Option<String>,
    pub null_ref: Option<String>,
}

pub struct RunEnv {
    pub ledger: PathBuf,
    pub lock: PathBuf,
    /// per-session `s<attempt>/` directories go here
    pub scratch: PathBuf,
    pub probe: Arc<dyn Probe + Send + Sync>,
    /// false skips `quiet_wait` (tests and test mode only)
    pub quiet: bool,
    /// adds `"test_mode": true` to the record
    pub test_mode: bool,
}

fn take_lock(path: &std::path::Path) -> Result<File, String> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    let f = OpenOptions::new().create(true).truncate(false).write(true).open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    // A short grace period: a process forked elsewhere in this process may briefly hold a copy of
    // a just-closed lock fd until its exec (O_CLOEXEC). A real holder (another run) fails fast.
    let t = std::time::Instant::now();
    while unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        if t.elapsed() >= LOCK_GRACE {
            return Err(format!("{} is held by another run", path.display()));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok(f) // released when the File drops (or the process dies)
}

fn now_unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Why the null record cannot serve this run, if it cannot.
fn null_problem(spec: &RunSpec, env: &RunEnv, null: &Value) -> Option<String> {
    let id = null["id"].as_str().unwrap_or("?");
    if null["kind"] != "null" {
        return Some(format!("{id} is a {} record, not a null", null["kind"]));
    }
    if null["status"] != "OK" {
        return Some(format!("null {id} has status {}", null["status"]));
    }
    if null["workload"].as_str() != Some(spec.workload.as_str()) {
        return Some(format!("null {id} is for workload {}, not {}", null["workload"], spec.workload));
    }
    if null["test_mode"] == true && !env.test_mode {
        return Some(format!("null {id} is a test-mode record"));
    }
    let Some(at) = null["at"].as_str().and_then(ledger::parse_iso_utc) else {
        return Some(format!("null {id} has no readable `at`"));
    };
    let age = now_unix().saturating_sub(at);
    if age > NULL_MAX_AGE_DAYS * 86_400 {
        return Some(format!("null {id} is {:.1} days old (max {NULL_MAX_AGE_DAYS} days)", age as f64 / 86_400.0));
    }
    if null["base"]["sha256"].as_str() != Some(spec.base.sha256.as_str()) {
        return Some(format!("null {id} was taken on base sha256 {}, not {}", null["base"]["sha256"], spec.base.sha256));
    }
    None
}

/// Checks made before any session. Ok(Err(reason)) means refuse; Ok(Ok(null record)) proceeds.
fn preflight(spec: &RunSpec, env: &RunEnv) -> Result<Result<Option<Value>, String>, String> {
    if spec.sessions == 0 {
        return Ok(Err("sessions must be at least 1".into()));
    }
    for a in [&spec.base, &spec.cand] {
        match sha256_file(&a.bin) {
            Ok(s) if s == a.sha256 => {}
            Ok(s) => return Ok(Err(format!("{} sha256 is {s}, not the given {}", a.bin.display(), a.sha256))),
            Err(e) => return Ok(Err(e)),
        }
    }
    if spec.kind == RunKind::Null {
        if spec.base.sha256 != spec.cand.sha256 {
            return Ok(Err("a null run needs byte-identical arms (sha256 differs)".into()));
        }
        let canon = |p: &PathBuf| std::fs::canonicalize(p).unwrap_or_else(|_| p.clone());
        if canon(&spec.base.bin) == canon(&spec.cand.bin) {
            return Ok(Err("a null run needs the two arms at different paths".into()));
        }
        if !spec.targets.is_empty() {
            return Ok(Err("a null run takes no targets (nullband recomputes any target set)".into()));
        }
        return Ok(Ok(None));
    }
    if spec.base.sha256 == spec.cand.sha256 {
        return Ok(Err(format!("base and cand have the same sha256 {}", spec.base.sha256)));
    }
    let Some(nid) = &spec.null_ref else {
        return Ok(Err(format!("a {} run needs --null", spec.kind.as_str())));
    };
    let Some(null) = ledger::read_all(&env.ledger)?.into_iter().find(|r| r["id"].as_str() == Some(nid.as_str())) else {
        return Ok(Err(format!("null {nid} is not in the ledger")));
    };
    if let Some(p) = null_problem(spec, env, &null) {
        return Ok(Err(p));
    }
    if let Err(e) = null_band(&null, &spec.workload, &spec.targets) {
        return Ok(Err(e));
    }
    Ok(Ok(Some(null)))
}

fn header(spec: &RunSpec, env: &RunEnv) -> Map<String, Value> {
    let mut m = Map::new();
    if env.test_mode {
        m.insert("test_mode".into(), json!(true));
    }
    if spec.kind == RunKind::Screen {
        m.insert("provisional".into(), json!(true));
    }
    m.insert("workload".into(), json!(spec.workload));
    m.insert("base".into(), json!(spec.base));
    m.insert("cand".into(), json!(spec.cand));
    m.insert("rule".into(), json!(spec.rule));
    m.insert("null_ref".into(), json!(spec.null_ref));
    m.insert("targets".into(), json!(spec.targets));
    m.insert("sessions_requested".into(), json!(spec.sessions));
    m.insert("protocol_sessions".into(), json!(spec.kind.protocol_sessions()));
    m.insert("protocol_deviation".into(), json!(spec.sessions != spec.kind.protocol_sessions()));
    m.insert("criterion_args".into(), json!(CRITERION_ARGS));
    m.insert("perf_events".into(), json!(PERF_EVENTS));
    m.insert("datasets_root".into(), json!(std::env::var("CQLITE_DATASETS_ROOT").ok()));
    m.insert("session_timeout_s".into(), json!(SESSION_TIMEOUT.as_secs()));
    m.insert("max_session_replacements".into(), json!(MAX_SESSION_REPLACEMENTS));
    m.insert("quiet_wait_skipped".into(), json!(!env.quiet));
    m.insert("scratch".into(), json!(env.scratch));
    m
}

/// Mutable run state, kept outside `measure` so a panic or error still records what was done.
struct State {
    rec: Map<String, Value>,
    sessions: Vec<SessionRecord>,
    completed: u32,
    rejected: u32,
}

/// `<scratch>/progress.json`, rewritten (atomically) after every session, for operators watching
/// a long run. Advisory only; the ledger record is the result.
fn write_progress(env: &RunEnv, spec: &RunSpec, st: &State) {
    let v = json!({
        "kind": spec.kind.as_str(), "sessions_requested": spec.sessions,
        "sessions_completed": st.completed, "sessions_rejected": st.rejected,
        "attempts": st.sessions.len(), "at_unix": now_unix(),
    });
    let tmp = env.scratch.join("progress.json.tmp");
    if std::fs::write(&tmp, v.to_string()).is_ok() {
        let _ = std::fs::rename(&tmp, env.scratch.join("progress.json"));
    }
}

enum End {
    Complete,
    NoiseGate(String),
    Aborted,
}

fn measure(spec: &RunSpec, env: &RunEnv, null: Option<&Value>, st: &mut State) -> Result<End, String> {
    let probe: &dyn Probe = &*env.probe;
    if env.quiet {
        let mut sleep = |d: Duration| {
            let mut left = d;
            while !left.is_zero() && !ABORT.load(Ordering::SeqCst) {
                let step = left.min(Duration::from_millis(200));
                std::thread::sleep(step);
                left -= step;
            }
        };
        let r = quiet_wait(probe, QUIET_HOLD, QUIET_MAX_LOAD, QUIET_TIMEOUT, &mut sleep);
        if ABORT.load(Ordering::SeqCst) {
            return Ok(End::Aborted);
        }
        match r {
            Ok(s) => {
                st.rec.insert("quiet_wait".into(), json!(s));
            }
            Err(e) => return Ok(End::NoiseGate(format!("quiet-host precondition: {e}"))),
        }
    }
    let core = choose_core(probe);
    st.rec.insert("host_start".into(), json!(snapshot(probe, Some(core))));
    st.rec.insert("core".into(), json!(core));
    let nonce = fresh_nonce();
    st.rec.insert("nonce".into(), json!(format!("{nonce:#018x}")));

    let mut expected: BTreeSet<String> = spec.targets.iter().cloned().collect();
    let mut attempt = 0u32;
    while st.completed < spec.sessions {
        if ABORT.load(Ordering::SeqCst) {
            return Ok(End::Aborted);
        }
        let seed = session_seed(st.completed, st.rejected, nonce);
        let inp = SessionInput {
            attempt,
            index: st.completed,
            replacements: st.rejected,
            seed,
            core,
            base: &spec.base,
            cand: &spec.cand,
            dir: env.scratch.join(format!("s{attempt}")),
            expected: &expected,
            probe: env.probe.clone(),
            abort: &ABORT,
            timeout: SESSION_TIMEOUT,
        };
        attempt += 1;
        let rec = match run_session(&inp)? {
            SessionOutcome::Aborted => return Ok(End::Aborted),
            SessionOutcome::Done(r) => r,
        };
        let rejected = rec.rejection.is_some();
        if !rejected && st.completed == 0 {
            expected.extend(rec.cases.keys().cloned());
        }
        st.sessions.push(rec);
        if rejected {
            st.rejected += 1;
        } else {
            st.completed += 1;
        }
        write_progress(env, spec, st);
        if st.rejected > MAX_SESSION_REPLACEMENTS {
            return Ok(End::NoiseGate(format!("{} sessions rejected (max {MAX_SESSION_REPLACEMENTS} replaced)", st.rejected)));
        }
    }

    let stored = serde_json::to_value(&st.sessions).map_err(|e| e.to_string())?;
    let maps = ratio_maps(&stored);
    let cases: Vec<String> = expected.into_iter().collect();
    st.rec.insert("cases".into(), json!(cases));
    let fields = match null {
        None => {
            let mut f = null_fields(&maps, &cases)?;
            let bw = f["b_workload"].as_f64().unwrap_or(f64::NAN);
            f.insert("null_band_over_max".into(), json!(bw > NULL_BAND_MAX));
            f
        }
        Some(n) => {
            let (bt, bw) = null_band(n, &spec.workload, &spec.targets)?;
            let mut f = verdict_fields(&maps, &cases, &spec.targets, bt, bw)?;
            let null_cases: BTreeSet<&str> = n["cases"].as_array().into_iter().flatten().filter_map(|v| v.as_str()).collect();
            let ours: BTreeSet<&str> = cases.iter().map(|s| s.as_str()).collect();
            f.insert("null_cases_match".into(), json!(null_cases == ours));
            f
        }
    };
    st.rec.extend(fields);
    Ok(End::Complete)
}

fn panic_text(p: &(dyn std::any::Any + Send)) -> String {
    p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_else(|| "panic".into())
}

/// Runs `spec` and appends exactly one ledger record; returns its id. `Err` only when the lock
/// cannot be taken, the ledger cannot be read or written, or (with no record) before the lock.
pub fn run(spec: &RunSpec, env: &RunEnv) -> Result<String, String> {
    let _lock = take_lock(&env.lock)?;
    let mut st = State { rec: header(spec, env), sessions: Vec::new(), completed: 0, rejected: 0 };
    let null = match preflight(spec, env)? {
        Err(reason) => {
            st.rec.insert("status".into(), json!("refused"));
            st.rec.insert("reason".into(), json!(reason));
            st.rec.insert("sessions".into(), json!([]));
            st.rec.insert("sessions_completed".into(), json!(0));
            st.rec.insert("sessions_rejected".into(), json!(0));
            return ledger::append(&env.ledger, spec.kind.as_str(), Value::Object(st.rec));
        }
        Ok(n) => n,
    };
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| measure(spec, env, null.as_ref(), &mut st)));
    let (status, reason) = match outcome {
        Ok(Ok(End::Complete)) => ("OK", None),
        Ok(Ok(End::NoiseGate(r))) => ("NOISE_GATE_FAILED", Some(r)),
        Ok(Ok(End::Aborted)) => ("aborted", Some("signal".to_string())),
        Ok(Err(e)) => ("aborted", Some(format!("error: {e}"))),
        Err(p) => ("aborted", Some(format!("panic: {}", panic_text(&*p)))),
    };
    if status != "OK" {
        // no partial statistics on a run that did not finish
        for k in ["verdict", "verdicts", "results", "intervals", "T", "b_targets", "b_workload"] {
            st.rec.remove(k);
        }
    }
    st.rec.insert("status".into(), json!(status));
    st.rec.insert("reason".into(), json!(reason));
    st.rec.insert("sessions".into(), serde_json::to_value(&st.sessions).map_err(|e| e.to_string())?);
    st.rec.insert("sessions_completed".into(), json!(st.completed));
    st.rec.insert("sessions_rejected".into(), json!(st.rejected));
    ledger::append(&env.ledger, spec.kind.as_str(), Value::Object(st.rec))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_round_trip() {
        for k in ["null", "calibration", "screen", "confirm", "crosscheck", "proposal"] {
            assert_eq!(RunKind::parse(k).unwrap().as_str(), k);
        }
        assert!(RunKind::parse("x").is_none());
        assert_eq!(RunKind::Screen.protocol_sessions(), 5);
        assert_eq!(RunKind::Crosscheck.protocol_sessions(), 10);
        assert_eq!(RunKind::Confirm.protocol_sessions(), 20);
    }

    #[test]
    fn lock_is_exclusive() {
        let d = std::env::temp_dir().join(format!("r2-lock-{}", std::process::id()));
        let p = d.join("l");
        let a = take_lock(&p).unwrap();
        assert!(take_lock(&p).is_err());
        drop(a);
        assert!(take_lock(&p).is_ok());
    }
}
