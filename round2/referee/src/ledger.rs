//! The append-only ledger: one JSON object per line. Nothing here edits or deletes a line.

use serde_json::{json, Map, Value};
use std::fs::OpenOptions;
use std::io::Write;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

const MACHSMITH_DIR: &str = "/home/dev/projects/machsmith";

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

fn iso_utc(secs: u64) -> String {
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
    fn iso_utc_known_dates() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_utc(1_794_528_000), "2026-11-13T00:00:00Z");
        assert_eq!(iso_utc(951_782_400 + 86_399), "2000-02-29T23:59:59Z");
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
