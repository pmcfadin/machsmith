//! The append-only ledger: one JSON object per line. Nothing here edits or deletes a line.

use serde_json::{json, Map, Value};
use std::fs::OpenOptions;
use std::io::Write;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

pub const MACHSMITH_DIR: &str = "/home/dev/projects/machsmith";

/// The real round 2 ledger, relative to `MACHSMITH_DIR`.
pub const REAL_LEDGER_REL: &str = "round2/results/ledger.jsonl";

/// Canonical form of a path whose file may not exist yet (its parent canonicalized).
fn canonical(p: &Path) -> PathBuf {
    if let Ok(c) = std::fs::canonicalize(p) {
        return c;
    }
    let parent = p.parent().map(|d| if d.as_os_str().is_empty() { Path::new(".") } else { d });
    match (parent.and_then(|d| std::fs::canonicalize(d).ok()), p.file_name()) {
        (Some(d), Some(f)) => d.join(f),
        _ => p.to_path_buf(),
    }
}

/// True when `p` resolves to the real round 2 ledger (test mode must never write there).
pub fn is_real_ledger(p: &Path) -> bool {
    canonical(p) == canonical(&Path::new(MACHSMITH_DIR).join(REAL_LEDGER_REL))
}

/// All records in order. A missing ledger is empty; blank lines are skipped; a bad line is an error.
pub fn read_all(path: &Path) -> Result<Vec<Value>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    text.lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
        .map(|(i, l)| serde_json::from_str(l).map_err(|e| format!("{}:{}: not JSON: {e}", path.display(), i + 1)))
        .collect()
}

fn lock_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".lock");
    PathBuf::from(s)
}

/// `YYYY-MM-DDTHH:MM:SSZ` for a Unix time.
pub fn iso_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
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

/// Inverse of `iso_utc` for its exact format; None for anything else.
pub fn parse_iso_utc(s: &str) -> Option<u64> {
    let b = s.as_bytes();
    if b.len() != 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' || b[16] != b':' || b[19] != b'Z' {
        return None;
    }
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, m, d) = (n(0..4)?, n(5..7)?, n(8..10)?);
    let (hh, mm, ss) = (n(11..13)?, n(14..16)?, n(17..19)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || hh > 23 || mm > 59 || ss > 59 {
        return None;
    }
    // days from civil (Howard Hinnant)
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + hh * 3600 + mm * 60 + ss;
    u64::try_from(secs).ok().filter(|&x| iso_utc(x) == s)
}

fn git(args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(MACHSMITH_DIR)
        .args(args)
        .output()
        .map_err(|e| format!("git {}: {e}", args.join(" ")))?;
    if !out.status.success() {
        return Err(format!("git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `{"commit", "dirty_paths"}` of the machsmith repo; on any git failure
/// `{"commit": null, "dirty_paths": null, "error": msg}`.
pub fn machsmith_state() -> Value {
    let r = (|| -> Result<Value, String> {
        let commit = git(&["rev-parse", "HEAD"])?.trim().to_string();
        let status = git(&["status", "--porcelain"])?;
        // "XY path" or "XY old -> new"; keep the path (the new one for renames).
        let paths: Vec<String> = status
            .lines()
            .filter(|l| l.len() > 3)
            .map(|l| {
                let p = &l[3..];
                p.rsplit_once(" -> ").map_or(p, |(_, n)| n).to_string()
            })
            .collect();
        Ok(json!({"commit": commit, "dirty_paths": paths}))
    })();
    r.unwrap_or_else(|e| json!({"commit": null, "dirty_paths": null, "error": e}))
}

fn id_number(v: &Value) -> Option<u64> {
    v["id"].as_str()?.strip_prefix("R2-")?.parse().ok()
}

/// Appends one record under an exclusive flock on `<path>.lock`; returns its id ("R2-000001").
/// `body` must be a JSON object; its fields follow `id`, `kind`, `at`, `machsmith`.
pub fn append(path: &Path, kind: &str, body: Value) -> Result<String, String> {
    let Value::Object(body) = body else { return Err("ledger: body must be a JSON object".into()) };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let lp = lock_path(path);
    let lock = OpenOptions::new().create(true).truncate(false).write(true).open(&lp).map_err(|e| format!("{}: {e}", lp.display()))?;
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(format!("flock {}: {}", lp.display(), std::io::Error::last_os_error()));
    }
    let existing = read_all(path)?;
    let mut max = 0;
    for r in &existing {
        max = max.max(id_number(r).ok_or_else(|| "ledger: a line has no valid R2- id; refusing to append".to_string())?);
    }
    let id = format!("R2-{:06}", max + 1);
    let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let mut rec = Map::new();
    rec.insert("id".into(), json!(id));
    rec.insert("kind".into(), json!(kind));
    rec.insert("at".into(), json!(iso_utc(at)));
    rec.insert("machsmith".into(), machsmith_state());
    for (k, v) in body {
        if !rec.contains_key(&k) {
            rec.insert(k, v);
        }
    }
    let mut line = serde_json::to_string(&Value::Object(rec)).map_err(|e| e.to_string())?;
    line.push('\n');
    let mut f = OpenOptions::new().create(true).append(true).open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    f.write_all(line.as_bytes()).map_err(|e| format!("{}: {e}", path.display()))?;
    f.sync_all().map_err(|e| format!("fsync {}: {e}", path.display()))?;
    Ok(id) // lock released when `lock` drops
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("r2-ledger-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.join("ledger.jsonl")
    }

    #[test]
    fn ledger_ids_increase() {
        let p = tmp("ids");
        assert_eq!(append(&p, "null", json!({"x": 1})).unwrap(), "R2-000001");
        let first = std::fs::read(&p).unwrap();
        assert_eq!(append(&p, "confirm", json!({"y": 2})).unwrap(), "R2-000002");
        let all = std::fs::read(&p).unwrap();
        assert!(all.starts_with(&first));
        assert_eq!(all.iter().filter(|b| **b == b'\n').count(), 2);
        let recs = read_all(&p).unwrap();
        assert_eq!(recs[1]["id"], "R2-000002");
        assert_eq!(recs[1]["y"], 2);
        assert!(recs[0]["at"].as_str().unwrap().ends_with('Z'));
        assert!(recs[0]["machsmith"].is_object());
    }

    #[test]
    fn real_ledger_detected() {
        assert!(is_real_ledger(Path::new("/home/dev/projects/machsmith/round2/results/../results/ledger.jsonl")));
        assert!(!is_real_ledger(&tmp("real")));
    }

    #[test]
    fn iso_utc_known_dates() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_utc(1_794_528_000), "2026-11-13T00:00:00Z");
        assert_eq!(iso_utc(951_782_400 + 86_399), "2000-02-29T23:59:59Z");
    }

    #[test]
    fn parse_iso_utc_round_trips() {
        for t in [0, 951_782_400 + 86_399, 1_794_528_000, 1_794_528_000 + 12_345] {
            assert_eq!(parse_iso_utc(&iso_utc(t)), Some(t));
        }
        assert_eq!(parse_iso_utc("2026-02-30T00:00:00Z"), None);
        assert_eq!(parse_iso_utc("2026-11-13 00:00:00"), None);
    }

    #[test]
    fn append_refuses_corrupt_ledger() {
        let p = tmp("bad");
        std::fs::write(&p, "not json\n").unwrap();
        assert!(append(&p, "k", json!({})).is_err());
    }

    #[test]
    fn machsmith_state_shape() {
        let s = machsmith_state();
        assert!(s.get("commit").is_some() && s.get("dirty_paths").is_some());
    }
}
