//! `r2`: the round 2 referee CLI. See round2/README.md.
//!
//! Environment: `R2_LEDGER` (or `--ledger`), `R2_LOCK`, `R2_SCRATCH` (optional), and
//! `R2_TEST_MODE=1` (skip the quiet wait, always-quiet probe, records marked `test_mode`;
//! refused on the real ledger).

use r2_referee::host::{quiet_wait, Probe, ProcProbe, QuietProbe, QUIET_HOLD, QUIET_MAX_LOAD};
use r2_referee::ledger;
use r2_referee::run::{null_band, run, RunEnv, RunKind, RunSpec, ABORT};
use r2_referee::session::{sha256_file, Arm};
use r2_referee::verdicts::{report, t_of};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

const WORKLOADS: [&str; 3] = ["read", "decode", "partition_lookup"];
const USAGE: &str = "usage:
  r2 quiet-wait [--hold-min 10] [--max-load 2.0] [--timeout-h 12]
  r2 run --kind <null|calibration|screen|confirm|crosscheck|proposal> --workload <read|decode|partition_lookup>
         --base <bin> --cand <bin> [--sessions N] [--targets <file.json>] [--rule <id>] [--null <ledger-id>] [--ledger <path>]
  r2 report <ledger-id> [--ledger <path>]
  r2 nullband --null <ledger-id> --workload W [--targets <file.json>] [--ledger <path>]
  r2 ledger-append --kind <k> --body-file <json-object-file> [--ledger <path>]";

extern "C" fn on_signal(_: libc::c_int) {
    ABORT.store(true, Ordering::SeqCst);
}

fn install_signal_handlers() {
    for sig in [libc::SIGINT, libc::SIGTERM] {
        unsafe {
            let mut sa: libc::sigaction = std::mem::zeroed();
            sa.sa_sigaction = on_signal as extern "C" fn(libc::c_int) as usize;
            libc::sigemptyset(&mut sa.sa_mask);
            libc::sigaction(sig, &sa, std::ptr::null_mut());
        }
    }
}

/// `--flag value` pairs plus positionals; every flag takes a value.
fn parse(args: &[String]) -> Result<(BTreeMap<String, String>, Vec<String>), String> {
    let (mut flags, mut pos) = (BTreeMap::new(), Vec::new());
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if let Some(k) = a.strip_prefix("--") {
            let v = it.next().ok_or_else(|| format!("--{k} needs a value"))?;
            if flags.insert(k.to_string(), v.clone()).is_some() {
                return Err(format!("--{k} given twice"));
            }
        } else {
            pos.push(a.clone());
        }
    }
    Ok((flags, pos))
}

fn allow(flags: &BTreeMap<String, String>, known: &[&str]) -> Result<(), String> {
    match flags.keys().find(|k| !known.contains(&k.as_str())) {
        Some(k) => Err(format!("unknown flag --{k}")),
        None => Ok(()),
    }
}

fn need<'a>(flags: &'a BTreeMap<String, String>, k: &str) -> Result<&'a str, String> {
    flags.get(k).map(|s| s.as_str()).ok_or_else(|| format!("--{k} is required"))
}

fn num<T: std::str::FromStr>(flags: &BTreeMap<String, String>, k: &str, default: T) -> Result<T, String> {
    flags.get(k).map_or(Ok(default), |v| v.parse().map_err(|_| format!("--{k}: bad number {v}")))
}

fn ledger_path(flags: &BTreeMap<String, String>) -> Result<PathBuf, String> {
    flags
        .get("ledger")
        .cloned()
        .or_else(|| std::env::var("R2_LEDGER").ok())
        .map(PathBuf::from)
        .ok_or_else(|| "no ledger: pass --ledger or set R2_LEDGER".into())
}

fn read_targets(flags: &BTreeMap<String, String>) -> Result<Vec<String>, String> {
    let Some(f) = flags.get("targets") else { return Ok(Vec::new()) };
    let text = std::fs::read_to_string(f).map_err(|e| format!("{f}: {e}"))?;
    serde_json::from_str::<Vec<String>>(&text).map_err(|e| format!("{f}: not a JSON array of strings: {e}"))
}

fn find(ledger_path: &Path, id: &str) -> Result<Value, String> {
    ledger::read_all(ledger_path)?.into_iter().find(|r| r["id"].as_str() == Some(id)).ok_or_else(|| format!("{id} is not in {}", ledger_path.display()))
}

fn arm(name: &str, p: &str) -> Result<Arm, String> {
    let bin = std::fs::canonicalize(p).map_err(|e| format!("{p}: {e}"))?;
    Ok(Arm { name: name.into(), sha256: sha256_file(&bin)?, bin })
}

fn cmd_run(flags: &BTreeMap<String, String>) -> Result<i32, String> {
    allow(flags, &["kind", "workload", "base", "cand", "sessions", "targets", "rule", "null", "ledger"])?;
    let test_mode = std::env::var("R2_TEST_MODE").as_deref() == Ok("1");
    let ledger_p = ledger_path(flags)?;
    if test_mode && ledger::is_real_ledger(&ledger_p) {
        return Err(format!("test mode (R2_TEST_MODE=1) is refused on the real ledger {}", ledger_p.display()));
    }
    let kind = RunKind::parse(need(flags, "kind")?).ok_or("bad --kind")?;
    let workload = need(flags, "workload")?;
    if !WORKLOADS.contains(&workload) {
        return Err(format!("--workload must be one of {WORKLOADS:?}"));
    }
    let lock = std::env::var("R2_LOCK").map_err(|_| "R2_LOCK is not set (source round2/env.sh)")?;
    let scratch = std::env::var("R2_SCRATCH").map(PathBuf::from).unwrap_or_else(|_| {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        std::env::temp_dir().join(format!("r2-scratch-{t}-{}", std::process::id()))
    });
    let spec = RunSpec {
        kind,
        workload: workload.into(),
        base: arm("base", need(flags, "base")?)?,
        cand: arm("cand", need(flags, "cand")?)?,
        sessions: num(flags, "sessions", kind.protocol_sessions())?,
        targets: read_targets(flags)?,
        rule: flags.get("rule").cloned(),
        null_ref: flags.get("null").cloned(),
    };
    let probe: Arc<dyn Probe + Send + Sync> = if test_mode { Arc::new(QuietProbe) } else { Arc::new(ProcProbe) };
    let env = RunEnv { ledger: ledger_p.clone(), lock: PathBuf::from(lock), scratch, probe, quiet: !test_mode,
        test_mode,
        datasets_root: std::env::var("CQLITE_DATASETS_ROOT").ok(),
    };
    install_signal_handlers();
    let id = run(&spec, &env)?;
    let status = find(&ledger_p, &id)?["status"].as_str().unwrap_or("?").to_string();
    println!("{id} {status}");
    Ok(if status == "OK" { 0 } else { 2 })
}

fn cmd_quiet_wait(flags: &BTreeMap<String, String>) -> Result<i32, String> {
    allow(flags, &["hold-min", "max-load", "timeout-h"])?;
    let hold = num(flags, "hold-min", QUIET_HOLD.as_secs() as f64 / 60.0)?;
    let max_load = num(flags, "max-load", QUIET_MAX_LOAD)?;
    let timeout = num(flags, "timeout-h", 12.0f64)?;
    let snap = quiet_wait(
        &ProcProbe,
        Duration::from_secs_f64(hold * 60.0),
        max_load,
        Duration::from_secs_f64(timeout * 3600.0),
        &mut |d| std::thread::sleep(d),
    )?;
    println!("{}", json!(snap));
    Ok(0)
}

fn cmd_nullband(flags: &BTreeMap<String, String>) -> Result<i32, String> {
    allow(flags, &["null", "workload", "targets", "ledger"])?;
    let rec = find(&ledger_path(flags)?, need(flags, "null")?)?;
    let (bt, bw) = null_band(&rec, need(flags, "workload")?, &read_targets(flags)?)?;
    println!("{}", json!({"b_targets": bt, "b_workload": bw, "T": t_of(bt, bw)}));
    Ok(0)
}

fn cmd_report(flags: &BTreeMap<String, String>, pos: &[String]) -> Result<i32, String> {
    allow(flags, &["ledger"])?;
    let [id] = pos else { return Err("report takes one ledger id".into()) };
    print!("{}", report(&find(&ledger_path(flags)?, id)?));
    Ok(0)
}

fn cmd_ledger_append(flags: &BTreeMap<String, String>) -> Result<i32, String> {
    allow(flags, &["kind", "body-file", "ledger"])?;
    let f = need(flags, "body-file")?;
    let text = std::fs::read_to_string(f).map_err(|e| format!("{f}: {e}"))?;
    let body: Value = serde_json::from_str(&text).map_err(|e| format!("{f}: not JSON: {e}"))?;
    println!("{}", ledger::append(&ledger_path(flags)?, need(flags, "kind")?, body)?);
    Ok(0)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let r = match args.split_first() {
        Some((cmd, rest)) => parse(rest).and_then(|(flags, pos)| match cmd.as_str() {
            "run" | "quiet-wait" | "nullband" | "ledger-append" if !pos.is_empty() => Err(format!("unexpected argument {}", pos[0])),
            "run" => cmd_run(&flags),
            "quiet-wait" => cmd_quiet_wait(&flags),
            "nullband" => cmd_nullband(&flags),
            "ledger-append" => cmd_ledger_append(&flags),
            "report" => cmd_report(&flags, &pos),
            _ => Err(USAGE.into()),
        }),
        None => Err(USAGE.into()),
    };
    match r {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("r2: {e}");
            std::process::exit(1);
        }
    }
}
