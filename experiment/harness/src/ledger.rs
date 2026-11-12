//! The append-only ledger: one JSON object per line in `experiment/results/ledger.jsonl`.
//!
//! Nothing here edits or deletes a line. `append` takes an exclusive `flock` on a lock file next
//! to the ledger, so concurrent appenders get distinct, consecutive ids.

use crate::build::{a_path, root, sha256_hex, Rung};
use serde_json::{json, Value};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::path::PathBuf;

/// Counted attempts per agent (spec section 8).
pub const COUNTED_ATTEMPTS: u32 = 10;
/// Submissions per agent, free failures included.
pub const MAX_SUBMISSIONS: u32 = 30;

/// `MSX_LEDGER` (tests), else `experiment/results/ledger.jsonl`.
pub fn ledger_path() -> PathBuf {
    match std::env::var_os("MSX_LEDGER") {
        Some(p) => PathBuf::from(p),
        None => root().join("experiment/results/ledger.jsonl"),
    }
}

fn lock_path() -> PathBuf {
    ledger_path().with_file_name("ledger.lock")
}

/// Exclusive `flock` on `path`, held until the returned file is dropped.
pub fn lock_exclusive(path: &std::path::Path) -> Result<File, String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let f = OpenOptions::new().create(true).truncate(false).write(true).open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(format!("flock {}: {}", path.display(), std::io::Error::last_os_error()));
    }
    Ok(f)
}

/// All records in order. A missing ledger is empty; blank lines are skipped. A line that is not
/// JSON is skipped here but makes `append` refuse (see `last_id`), so corruption cannot go unseen.
pub fn read_all() -> Vec<Value> {
    let Ok(text) = std::fs::read_to_string(ledger_path()) else { return Vec::new() };
    text.lines().filter(|l| !l.trim().is_empty()).filter_map(|l| serde_json::from_str(l).ok()).collect()
}

fn last_id(text: &str) -> Result<u64, String> {
    let Some(line) = text.lines().rev().find(|l| !l.trim().is_empty()) else { return Ok(0) };
    let v: Value = serde_json::from_str(line).map_err(|e| format!("ledger: last line is not JSON ({e}); refusing to append"))?;
    v["id"]
        .as_str()
        .and_then(|s| s.strip_prefix('L'))
        .and_then(|s| s.parse::<u64>().ok())
        .ok_or_else(|| "ledger: last line has no valid id; refusing to append".to_string())
}

fn git(args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git").arg("-C").arg(root()).args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Seconds since the epoch as RFC 3339 UTC.
pub fn rfc3339(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // civil-from-days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

fn host() -> Value {
    let cpu = std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|t| t.lines().find(|l| l.starts_with("model name")).and_then(|l| l.split_once(':')).map(|(_, v)| v.trim().to_string()));
    let rel = std::fs::read_to_string("/proc/sys/kernel/osrelease").ok().map(|s| s.trim().to_string());
    // The measurement core is chosen per run among cpus 2..=15 (FIXED_CORE as fallback); record the
    // governor of the fallback core, and the set of governors over the candidate cores.
    let gov = |c: usize| std::fs::read_to_string(format!("/sys/devices/system/cpu/cpu{c}/cpufreq/scaling_governor")).ok().map(|s| s.trim().to_string());
    let mut set: Vec<String> = (2..=15).filter_map(gov).collect();
    set.sort();
    set.dedup();
    let load = std::fs::read_to_string("/proc/loadavg").ok().and_then(|t| t.split_whitespace().next().and_then(|x| x.parse::<f64>().ok()));
    json!({
        "cpu_model": cpu, "kernel_release": rel,
        "governor": gov(crate::measure::FIXED_CORE), "governor_core": crate::measure::FIXED_CORE,
        "governors_cpu2_15": set, "load_avg_1m": load,
    })
}

fn a_hashes() -> Value {
    let mut m = serde_json::Map::new();
    for (name, rung) in [("r1", Rung::R1), ("r2", Rung::R2), ("r3", Rung::R3), ("r3copy", Rung::R3Copy)] {
        let p = a_path(rung);
        if p.is_file() {
            if let Ok(h) = sha256_hex(&p) {
                m.insert(name.into(), h.into());
            }
        }
    }
    Value::Object(m)
}

/// Appends `record` (a JSON object) with the common fields filled in by this function; any
/// `id`, `ts`, `git_head`, `git_dirty`, `host`, `tool_sha256` or `a_sha256` in `record` is replaced.
/// Returns the new id.
pub fn append(record: Value) -> Result<String, String> {
    let Value::Object(mut obj) = record else { return Err("ledger record must be a JSON object".into()) };
    let _lock = lock_exclusive(&lock_path())?;
    let path = ledger_path();
    let existing = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let id = format!("L{:06}", last_id(&existing)? + 1);
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|e| e.to_string())?.as_secs();
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let tool = sha256_hex(&exe).map_err(|e| e.to_string())?;
    obj.insert("id".into(), id.clone().into());
    obj.insert("ts".into(), rfc3339(secs).into());
    obj.insert("git_head".into(), git(&["rev-parse", "HEAD"]).map(|s| s.trim().to_string()).into());
    obj.insert("git_dirty".into(), git(&["status", "--porcelain"]).map(|s| !s.trim().is_empty()).into());
    obj.insert("host".into(), host());
    obj.insert("tool_sha256".into(), tool.into());
    obj.insert("a_sha256".into(), a_hashes());
    let mut line = serde_json::to_string(&Value::Object(obj)).map_err(|e| e.to_string())?;
    line.push('\n');
    // A ledger that does not end in a newline would glue two records together.
    if !existing.is_empty() && !existing.ends_with('\n') {
        line.insert(0, '\n');
    }
    let mut f = OpenOptions::new().create(true).append(true).open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    f.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
    f.sync_all().map_err(|e| e.to_string())?;
    Ok(id)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    pub submissions: u32,
    pub counted: u32,
}

/// Submissions and counted attempts so far for `(kernel, arm)`. Only `attempt` records count:
/// free failures and noise failures are submissions; `refused` records are neither. A record counts
/// as a counted attempt when its `counted_attempt` is a number.
pub fn budget(records: &[Value], kernel: &str, arm: &str) -> Budget {
    let mut b = Budget { submissions: 0, counted: 0 };
    for r in records {
        if r["kind"] == "attempt" && r["kernel"] == kernel && r["arm"] == arm {
            b.submissions += 1;
            if r["counted_attempt"].is_number() {
                b.counted += 1;
            }
        }
    }
    b
}
