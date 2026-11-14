//! Host gate (quiet wait), per-session noise monitor. Spec section 6.4.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const MAX_SESSION_LOAD: f64 = 4.0; // spec 6.4 amended 2026-11-13 (owner-approved, DECISIONS R18)
pub const MAX_SIBLING_UTIL: f64 = 0.20;
/// Spec 6.4 (amended, DECISIONS R15): max share of an arm's wall time spent on the pinned core by
/// anything other than that arm.
pub const MAX_FOREIGN_CPU_FRAC: f64 = 0.02;
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

/// A probe that always reports an idle host: no load, no cargo/rustc, no CPU times (so no
/// sibling or foreign-CPU figure). Test mode only (`R2_TEST_MODE=1`); its
/// records carry `"test_mode": true`.
pub struct QuietProbe;

impl Probe for QuietProbe {
    fn loadavg(&self) -> (f64, f64, f64) {
        (0.0, 0.0, 0.0)
    }
    fn cargo_rustc_pids(&self) -> Vec<u32> {
        Vec::new()
    }
    fn cpu_times(&self) -> BTreeMap<usize, (u64, u64)> {
        BTreeMap::new()
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
    ForeignCpu(f64),
    ProcessFailed(String),
    MissingCase(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct MonitorSample {
    pub load1: f64,
    pub cargo_rustc_pids: Vec<u32>,
}

/// The session rejection rules (spec 6.4), pure. `session_sibling_util` is the SMT sibling's
/// utilisation over the whole session; `foreign_fracs` = each bench arm's foreign-CPU fraction
/// (`None` = not measurable, rule does not apply). First violated rule wins: load, cargo/rustc,
/// sibling, foreign CPU (the larger arm fraction is reported).
pub fn judge(samples: &[MonitorSample], session_sibling_util: Option<f64>, foreign_fracs: &[Option<f64>]) -> Option<Rejection> {
    if let Some(s) = samples.iter().find(|s| s.load1 > MAX_SESSION_LOAD) {
        return Some(Rejection::Load(s.load1));
    }
    if let Some(s) = samples.iter().find(|s| !s.cargo_rustc_pids.is_empty()) {
        return Some(Rejection::CargoRustc(s.cargo_rustc_pids.clone()));
    }
    if let Some(u) = session_sibling_util.filter(|u| *u > MAX_SIBLING_UTIL) {
        return Some(Rejection::SiblingUtil(u));
    }
    let worst = foreign_fracs.iter().flatten().copied().fold(f64::NEG_INFINITY, f64::max);
    if worst > MAX_FOREIGN_CPU_FRAC {
        return Some(Rejection::ForeignCpu(worst));
    }
    None
}

/// Foreign CPU of one arm: busy time of the pinned core over the arm's run (jiffies delta over
/// `clk_tck`) minus the arm's own CPU time, floored at 0, over its wall time. `None` when a
/// snapshot lacks the core, the busy counter went backwards, `clk_tck` is 0 or `wall_s <= 0`.
pub fn foreign_fraction(
    busy_before: Option<u64>,
    busy_after: Option<u64>,
    clk_tck: u64,
    own_cpu_s: f64,
    wall_s: f64,
) -> Option<f64> {
    let (b0, b1) = (busy_before?, busy_after?);
    if wall_s <= 0.0 || clk_tck == 0 || b1 < b0 {
        return None;
    }
    let busy_s = (b1 - b0) as f64 / clk_tck as f64;
    Some((busy_s - own_cpu_s).max(0.0) / wall_s)
}

/// `nonvoluntary_ctxt_switches` from /proc/<pid>/status; None when unreadable or absent.
pub fn nonvoluntary_ctxt_switches(pid: u32) -> Option<u64> {
    let t = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    t.lines().find_map(|l| l.strip_prefix("nonvoluntary_ctxt_switches:")).and_then(|v| v.trim().parse().ok())
}

fn take_sample(p: &dyn Probe) -> MonitorSample {
    MonitorSample { load1: p.loadavg().0, cargo_rustc_pids: p.cargo_rustc_pids() }
}

/// A 1 s load/cargo sampling thread around one session, plus one session-wide utilisation
/// figure for the SMT sibling (core + 16) from the first and last `cpu_times()` snapshots.
pub struct SessionMonitor {
    probe: Arc<dyn Probe + Send + Sync>,
    sibling: usize,
    first: BTreeMap<usize, (u64, u64)>,
    stop: Arc<AtomicBool>,
    samples: Arc<Mutex<Vec<MonitorSample>>>,
    thread: Option<std::thread::JoinHandle<()>>,
    foreign: Vec<Option<f64>>,
}

impl SessionMonitor {
    pub fn start(probe: Arc<dyn Probe + Send + Sync>, core: usize) -> SessionMonitor {
        let stop = Arc::new(AtomicBool::new(false));
        let samples = Arc::new(Mutex::new(Vec::new()));
        let first = probe.cpu_times();
        let (st, sa, pr) = (stop.clone(), samples.clone(), probe.clone());
        let thread = std::thread::spawn(move || loop {
            // sleep 1 s in short slices so stopping returns promptly
            for _ in 0..20 {
                if st.load(Ordering::Relaxed) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            sa.lock().unwrap().push(take_sample(&*pr));
        });
        SessionMonitor { probe, sibling: core + SIBLING_OFFSET, first, stop, samples, thread: Some(thread), foreign: Vec::new() }
    }

    /// Records one arm's foreign-CPU fraction (see `foreign_fraction`).
    pub fn add_process(&mut self, foreign_frac: Option<f64>) {
        self.foreign.push(foreign_frac);
    }

    fn halt(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }

    pub fn finish(mut self) -> Option<Rejection> {
        self.halt();
        // a last sample so sessions shorter than 1 s still get a load/cargo check
        let last = take_sample(&*self.probe);
        let last_times = self.probe.cpu_times();
        let mut samples = self.samples.lock().unwrap().clone();
        samples.push(last);
        let sib = util(&self.first, &last_times, self.sibling);
        judge(&samples, sib, &self.foreign)
    }
}

impl Drop for SessionMonitor {
    fn drop(&mut self) {
        self.halt();
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

    fn sample(load1: f64, pids: Vec<u32>) -> MonitorSample {
        MonitorSample { load1, cargo_rustc_pids: pids }
    }

    #[test]
    fn monitor_rejects_load() {
        assert_eq!(judge(&[sample(4.1, vec![])], None, &[]), Some(Rejection::Load(4.1)));
        assert_eq!(judge(&[sample(2.9, vec![])], None, &[]), None);
    }

    #[test]
    fn monitor_rejects_cargo() {
        let s = [sample(0.1, vec![]), sample(0.1, vec![7])];
        assert_eq!(judge(&s, None, &[]), Some(Rejection::CargoRustc(vec![7])));
    }

    #[test]
    fn monitor_rejects_sibling() {
        assert_eq!(judge(&[], Some(0.25), &[]), Some(Rejection::SiblingUtil(0.25)));
        assert_eq!(judge(&[], Some(0.19), &[]), None);
    }

    #[test]
    fn foreign_cpu_rule() {
        assert_eq!(judge(&[], None, &[Some(0.01), Some(0.03)]), Some(Rejection::ForeignCpu(0.03)));
        assert_eq!(judge(&[], None, &[Some(0.01), None]), None);
        assert_eq!(judge(&[], None, &[Some(0.02)]), None);
        assert_eq!(judge(&[sample(4.1, vec![])], None, &[Some(0.5)]), Some(Rejection::Load(4.1)));
        assert_eq!(judge(&[sample(0.1, vec![4])], None, &[Some(0.5)]), Some(Rejection::CargoRustc(vec![4])));
        assert_eq!(judge(&[], Some(0.25), &[Some(0.5)]), Some(Rejection::SiblingUtil(0.25)));
    }

    #[test]
    fn foreign_fraction_cases() {
        assert_eq!(foreign_fraction(Some(1000), Some(4000), 100, 29.0, 30.0), Some(1.0 / 30.0));
        assert_eq!(foreign_fraction(Some(1000), Some(1100), 100, 5.0, 30.0), Some(0.0));
        assert_eq!(foreign_fraction(None, Some(4000), 100, 29.0, 30.0), None);
        assert_eq!(foreign_fraction(Some(1000), None, 100, 29.0, 30.0), None);
        assert_eq!(foreign_fraction(Some(1000), Some(4000), 100, 29.0, 0.0), None);
        assert_eq!(foreign_fraction(Some(4000), Some(1000), 100, 1.0, 30.0), None);
    }

    #[test]
    fn session_load_limit_is_four() {
        assert_eq!(judge(&[sample(3.9, vec![])], None, &[]), None);
        assert_eq!(judge(&[sample(4.0, vec![])], None, &[]), None);
        assert_eq!(judge(&[sample(4.01, vec![])], None, &[]), Some(Rejection::Load(4.01)));
    }

    #[test]
    fn util_from_two_maps() {
        let a = BTreeMap::from([(21, (100, 1000)), (5, (0, 1000))]);
        let b = BTreeMap::from([(21, (300, 2000)), (5, (0, 1000))]);
        assert_eq!(util(&a, &b, 21), Some(0.2));
        assert_eq!(util(&a, &b, 5), None); // no elapsed jiffies
        assert_eq!(util(&a, &b, 99), None);
    }

    struct ScriptedProbe {
        times: Mutex<Vec<BTreeMap<usize, (u64, u64)>>>,
        pids: Vec<u32>,
    }
    impl Probe for ScriptedProbe {
        fn loadavg(&self) -> (f64, f64, f64) {
            (0.1, 0.0, 0.0)
        }
        fn cargo_rustc_pids(&self) -> Vec<u32> {
            self.pids.clone()
        }
        fn cpu_times(&self) -> BTreeMap<usize, (u64, u64)> {
            self.times.lock().unwrap().remove(0)
        }
    }

    fn scripted(sib_busy: u64, pids: Vec<u32>) -> Arc<ScriptedProbe> {
        // first call (start) then last call (finish); core 5 -> sibling cpu 21
        Arc::new(ScriptedProbe {
            times: Mutex::new(vec![
                BTreeMap::from([(5, (0, 0)), (21, (0, 0))]),
                BTreeMap::from([(5, (900, 1000)), (21, (sib_busy, 1000))]),
            ]),
            pids,
        })
    }

    #[test]
    fn monitor_session_wide_sibling() {
        assert_eq!(SessionMonitor::start(scripted(250, vec![]), 5).finish(), Some(Rejection::SiblingUtil(0.25)));
        assert_eq!(SessionMonitor::start(scripted(150, vec![]), 5).finish(), None);
    }

    #[test]
    fn monitor_short_session_gets_final_sample() {
        assert_eq!(SessionMonitor::start(scripted(0, vec![9]), 5).finish(), Some(Rejection::CargoRustc(vec![9])));
    }

    #[test]
    fn monitor_drop_stops_thread() {
        let m = SessionMonitor::start(scripted(0, vec![]), 5);
        let t = std::time::Instant::now();
        drop(m); // must join promptly
        assert!(t.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn proc_probe_reads_this_host() {
        let p = ProcProbe;
        assert!(p.loadavg().0.is_finite());
        assert!(!p.cpu_times().is_empty());
        assert!(nonvoluntary_ctxt_switches(std::process::id()).is_some());
    }
}
