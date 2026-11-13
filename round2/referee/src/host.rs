//! Host gate (quiet wait), per-session noise monitor. Spec section 6.4.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const MAX_SESSION_LOAD: f64 = 3.0;
pub const MAX_SIBLING_UTIL: f64 = 0.20;
pub const MAX_CTX_SWITCHES_PER_SEC: f64 = 50.0;
pub const QUIET_HOLD: Duration = Duration::from_secs(10 * 60);
pub const QUIET_MAX_LOAD: f64 = 2.0;
pub const QUIET_SAMPLE_EVERY: Duration = Duration::from_secs(60);
pub const SIBLING_OFFSET: usize = 16;
pub const FIXED_CORE: usize = 5;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct HostSnapshot {
    pub at_unix: u64,
    pub load1: f64,
    pub load5: f64,
    pub load15: f64,
    pub cargo_rustc_pids: Vec<u32>,
    pub core: Option<usize>,
}

pub trait Probe {
    fn loadavg(&self) -> (f64, f64, f64);
    fn cargo_rustc_pids(&self) -> Vec<u32>;
    /// cpu index -> (busy jiffies, total jiffies)
    fn cpu_times(&self) -> BTreeMap<usize, (u64, u64)>;
}

/// The real /proc implementation.
pub struct ProcProbe;

impl Probe for ProcProbe {
    fn loadavg(&self) -> (f64, f64, f64) {
        let t = std::fs::read_to_string("/proc/loadavg").unwrap_or_default();
        let mut it = t.split_whitespace().map(|x| x.parse::<f64>().unwrap_or(f64::INFINITY));
        (it.next().unwrap_or(f64::INFINITY), it.next().unwrap_or(f64::INFINITY), it.next().unwrap_or(f64::INFINITY))
    }

    fn cargo_rustc_pids(&self) -> Vec<u32> {
        let mut v = Vec::new();
        let Ok(rd) = std::fs::read_dir("/proc") else { return v };
        for e in rd.flatten() {
            let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else { continue };
            if let Ok(c) = std::fs::read_to_string(e.path().join("comm")) {
                let c = c.trim_end_matches('\n');
                if c == "cargo" || c == "rustc" {
                    v.push(pid);
                }
            }
        }
        v.sort_unstable();
        v
    }

    fn cpu_times(&self) -> BTreeMap<usize, (u64, u64)> {
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
}

pub fn util(before: &BTreeMap<usize, (u64, u64)>, after: &BTreeMap<usize, (u64, u64)>, cpu: usize) -> Option<f64> {
    let (b0, t0) = before.get(&cpu)?;
    let (b1, t1) = after.get(&cpu)?;
    let dt = t1.checked_sub(*t0)?;
    if dt == 0 {
        return None;
    }
    Some(b1.saturating_sub(*b0) as f64 / dt as f64)
}

fn now_unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn snapshot(p: &dyn Probe, core: Option<usize>) -> HostSnapshot {
    let (load1, load5, load15) = p.loadavg();
    HostSnapshot { at_unix: now_unix(), load1, load5, load15, cargo_rustc_pids: p.cargo_rustc_pids(), core }
}

/// Waits until no cargo/rustc process exists and the 1-minute load is below `max_load`
/// (strictly), continuously for `hold`. Samples once per 60 s via `sleep`; time is the sum of
/// the sleeps, so tests can simulate it. Errors with "timeout" once `timeout` of waiting elapsed.
pub fn quiet_wait(
    p: &dyn Probe,
    hold: Duration,
    max_load: f64,
    timeout: Duration,
    sleep: &mut dyn FnMut(Duration),
) -> Result<HostSnapshot, String> {
    let mut elapsed = Duration::ZERO;
    let mut quiet_since: Option<Duration> = None;
    loop {
        let snap = snapshot(p, None);
        let quiet = snap.load1 < max_load && snap.cargo_rustc_pids.is_empty();
        if quiet {
            let since = *quiet_since.get_or_insert(elapsed);
            if elapsed - since >= hold {
                return Ok(snap);
            }
        } else {
            quiet_since = None;
        }
        if elapsed >= timeout {
            return Err(format!(
                "timeout: host not quiet for {} s within {} s (load1 {:.2}, cargo/rustc pids {:?})",
                hold.as_secs(),
                timeout.as_secs(),
                snap.load1,
                snap.cargo_rustc_pids
            ));
        }
        sleep(QUIET_SAMPLE_EVERY);
        elapsed += QUIET_SAMPLE_EVERY;
    }
}

/// Round 1's rule: of cores 2..=15, the one whose own and SMT-sibling (core + 16) utilisation over
/// a 500 ms window is lowest; `FIXED_CORE` when /proc/stat lacks any of them.
pub fn choose_core(p: &dyn Probe) -> usize {
    let before = p.cpu_times();
    std::thread::sleep(Duration::from_millis(500));
    let after = p.cpu_times();
    let mut best: Option<(f64, usize)> = None;
    for c in 2..=15usize {
        match (util(&before, &after, c), util(&before, &after, c + SIBLING_OFFSET)) {
            (Some(a), Some(b)) => {
                if best.map_or(true, |(u, _)| a + b < u) {
                    best = Some((a + b, c));
                }
            }
            _ => return FIXED_CORE,
        }
    }
    best.map_or(FIXED_CORE, |(_, c)| c)
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub enum Rejection {
    Load(f64),
    CargoRustc(Vec<u32>),
    SiblingUtil(f64),
    CtxSwitches(f64),
    ProcessFailed(String),
    MissingCase(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct MonitorSample {
    pub load1: f64,
    pub cargo_rustc_pids: Vec<u32>,
    pub sibling_util: Option<f64>,
}

/// The session rejection rules (spec 6.4), pure. `procs` = (involuntary switches, run seconds)
/// per bench process. The first violated rule wins: load, cargo/rustc, sibling, switches.
pub fn judge(samples: &[MonitorSample], procs: &[(u64, f64)]) -> Option<Rejection> {
    if let Some(s) = samples.iter().find(|s| s.load1 > MAX_SESSION_LOAD) {
        return Some(Rejection::Load(s.load1));
    }
    if let Some(s) = samples.iter().find(|s| !s.cargo_rustc_pids.is_empty()) {
        return Some(Rejection::CargoRustc(s.cargo_rustc_pids.clone()));
    }
    if let Some(u) = samples.iter().filter_map(|s| s.sibling_util).find(|u| *u > MAX_SIBLING_UTIL) {
        return Some(Rejection::SiblingUtil(u));
    }
    for &(sw, secs) in procs {
        if secs > 0.0 {
            let rate = sw as f64 / secs;
            if rate > MAX_CTX_SWITCHES_PER_SEC {
                return Some(Rejection::CtxSwitches(rate));
            }
        }
    }
    None
}

/// `nonvoluntary_ctxt_switches` from /proc/<pid>/status; None when unreadable or absent.
pub fn nonvoluntary_ctxt_switches(pid: u32) -> Option<u64> {
    let t = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    t.lines().find_map(|l| l.strip_prefix("nonvoluntary_ctxt_switches:")).and_then(|v| v.trim().parse().ok())
}

/// A 1 s sampling thread around one session.
pub struct SessionMonitor {
    stop: Arc<AtomicBool>,
    samples: Arc<Mutex<Vec<MonitorSample>>>,
    thread: Option<std::thread::JoinHandle<()>>,
    procs: Vec<(u64, f64)>,
}

impl SessionMonitor {
    pub fn start(probe: Arc<dyn Probe + Send + Sync>, core: usize) -> SessionMonitor {
        let stop = Arc::new(AtomicBool::new(false));
        let samples = Arc::new(Mutex::new(Vec::new()));
        let (st, sa) = (stop.clone(), samples.clone());
        let thread = std::thread::spawn(move || {
            let sib = core + SIBLING_OFFSET;
            let mut prev = probe.cpu_times();
            loop {
                // sleep 1 s in short slices so finish() returns promptly
                for _ in 0..20 {
                    if st.load(Ordering::Relaxed) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                let cur = probe.cpu_times();
                let s = MonitorSample {
                    load1: probe.loadavg().0,
                    cargo_rustc_pids: probe.cargo_rustc_pids(),
                    sibling_util: util(&prev, &cur, sib),
                };
                prev = cur;
                sa.lock().unwrap().push(s);
            }
        });
        SessionMonitor { stop, samples, thread: Some(thread), procs: Vec::new() }
    }

    pub fn add_process(&mut self, _pid: u32, nonvoluntary_switches: u64, run_secs: f64) {
        self.procs.push((nonvoluntary_switches, run_secs));
    }

    pub fn finish(mut self) -> Option<Rejection> {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        let samples = self.samples.lock().unwrap().clone();
        judge(&samples, &self.procs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    struct FakeProbe {
        minute: Rc<Cell<u32>>,
        loaded: Box<dyn Fn(u32) -> (f64, Vec<u32>)>,
    }
    impl Probe for FakeProbe {
        fn loadavg(&self) -> (f64, f64, f64) {
            ((self.loaded)(self.minute.get()).0, 0.0, 0.0)
        }
        fn cargo_rustc_pids(&self) -> Vec<u32> {
            (self.loaded)(self.minute.get()).1
        }
        fn cpu_times(&self) -> BTreeMap<usize, (u64, u64)> {
            BTreeMap::new()
        }
    }

    fn run_wait(loaded: Box<dyn Fn(u32) -> (f64, Vec<u32>)>, timeout_min: u64) -> (Result<HostSnapshot, String>, u32) {
        let minute = Rc::new(Cell::new(0u32));
        let p = FakeProbe { minute: minute.clone(), loaded };
        let (m2, mut sleeps) = (minute.clone(), 0u32);
        let r = quiet_wait(&p, QUIET_HOLD, QUIET_MAX_LOAD, Duration::from_secs(timeout_min * 60), &mut |d| {
            assert_eq!(d, Duration::from_secs(60));
            sleeps += 1;
            m2.set(m2.get() + 1);
        });
        (r, sleeps)
    }

    #[test]
    fn quiet_wait_needs_full_hold() {
        let (r, sleeps) = run_wait(Box::new(|m| if m == 9 { (0.1, vec![42]) } else { (0.1, vec![]) }), 120);
        assert!(r.is_ok(), "{r:?}");
        assert!(sleeps >= 19, "sleeps {sleeps}");
    }

    #[test]
    fn quiet_wait_load_boundary_is_strict() {
        let (r, _) = run_wait(Box::new(|_| (2.0, vec![])), 30);
        assert!(r.is_err());
    }

    #[test]
    fn quiet_wait_times_out() {
        let (r, _) = run_wait(Box::new(|_| (5.0, vec![])), 30);
        assert!(r.unwrap_err().contains("timeout"));
    }

    fn sample(load1: f64, pids: Vec<u32>, sib: Option<f64>) -> MonitorSample {
        MonitorSample { load1, cargo_rustc_pids: pids, sibling_util: sib }
    }

    #[test]
    fn monitor_rejects_load() {
        assert_eq!(judge(&[sample(3.1, vec![], None)], &[]), Some(Rejection::Load(3.1)));
        assert_eq!(judge(&[sample(2.9, vec![], None)], &[]), None);
    }

    #[test]
    fn monitor_rejects_cargo() {
        let s = [sample(0.1, vec![], None), sample(0.1, vec![7], None)];
        assert_eq!(judge(&s, &[]), Some(Rejection::CargoRustc(vec![7])));
    }

    #[test]
    fn monitor_rejects_sibling() {
        assert_eq!(judge(&[sample(0.1, vec![], Some(0.25))], &[]), Some(Rejection::SiblingUtil(0.25)));
        assert_eq!(judge(&[sample(0.1, vec![], Some(0.19))], &[]), None);
    }

    #[test]
    fn ctx_switch_rate() {
        assert_eq!(judge(&[], &[(600, 10.0)]), Some(Rejection::CtxSwitches(60.0)));
        assert_eq!(judge(&[], &[(400, 10.0)]), None);
    }

    #[test]
    fn proc_probe_reads_this_host() {
        let p = ProcProbe;
        assert!(p.loadavg().0.is_finite());
        assert!(!p.cpu_times().is_empty());
        assert!(nonvoluntary_ctxt_switches(std::process::id()).is_some());
    }

    #[test]
    fn real_monitor_finishes() {
        let mut m = SessionMonitor::start(Arc::new(ProcProbe), 5);
        m.add_process(1, 0, 1.0);
        let _ = m.finish();
    }
}
