//! End-to-end tests of the referee run loop against a fake Criterion bench
//! (`tests/fixtures/fake_bench.sh`), through the real `taskset … perf stat …` wrapper.
//! Every test uses its own temporary ledger, lock and scratch; none touches the real ledger.

use r2_referee::host::{Probe, SIBLING_OFFSET};
use r2_referee::ledger;
use r2_referee::run::{manifest_path, null_band, run, RunEnv, RunKind, RunSpec, DATASETS_ROOT};
use r2_referee::session::{sha256_file, Arm};
use serde_json::{json, Value};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const REAL_LEDGER: &str = "/home/dev/projects/machsmith/round2/results/ledger.jsonl";

fn tmpdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("r2-run-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn fake_bench() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_bench.sh")
}

/// A wrapper script that sets `vars` and execs the fake bench; it is one arm's "binary".
fn arm(dir: &Path, file: &str, name: &str, vars: &[(&str, &str)]) -> Arm {
    let mut s = String::from("#!/bin/sh\n");
    for (k, v) in vars {
        s.push_str(&format!("export {k}='{v}'\n"));
    }
    s.push_str(&format!("exec '{}' \"$@\"\n", fake_bench().display()));
    let p = dir.join(file);
    std::fs::write(&p, s).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    Arm { name: name.into(), bin: p.clone(), sha256: sha256_file(&p).unwrap() }
}

/// An always-quiet probe (like test mode's `QuietProbe`) that also steers `choose_core` to `core`
/// (it and its SMT sibling idle, every other CPU busy), so the record's core can be checked.
/// `extra_switches` is added to every measured involuntary-switch count (0 = report quiet).
struct CoreProbe {
    core: usize,
    calls: AtomicU64,
    extra_switches: Option<u64>,
}

impl Probe for CoreProbe {
    fn loadavg(&self) -> (f64, f64, f64) {
        (0.0, 0.0, 0.0)
    }
    fn cargo_rustc_pids(&self) -> Vec<u32> {
        Vec::new()
    }
    fn cpu_times(&self) -> BTreeMap<usize, (u64, u64)> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst) * 100;
        (0..32)
            .map(|c| (c, if c == self.core || c == self.core + SIBLING_OFFSET { (0, n) } else { (n, n) }))
            .collect()
    }
    fn ctx_switches(&self, measured: u64) -> u64 {
        self.extra_switches.map_or(0, |x| measured + x)
    }
}

fn env_in(dir: &Path, core: usize) -> RunEnv {
    RunEnv {
        ledger: dir.join("ledger.jsonl"),
        lock: dir.join("r2.lock"),
        scratch: dir.join("scratch"),
        probe: Arc::new(CoreProbe { core, calls: AtomicU64::new(0), extra_switches: None }),
        quiet: false,
        test_mode: true,
        datasets_root: Some(DATASETS_ROOT.into()),
    }
}

/// Writes `<bin>.meta.json` as round2/build.sh would; `sha` overrides the true sha256.
fn write_manifest(a: &Arm, sha: Option<&str>) {
    let m = json!({
        "cqlite_commit": "7f9abe3663254724439dbe71aff77bb93faa1c2d", "branch": null, "bench": "read",
        "features": "", "sha256": sha.unwrap_or(&a.sha256), "built_at": "2026-11-13T00:00:00Z",
    });
    std::fs::write(manifest_path(&a.bin), m.to_string()).unwrap();
}

/// A non-test-mode environment (still a temp ledger, no quiet wait).
fn real_env_in(dir: &Path, core: usize) -> RunEnv {
    RunEnv { test_mode: false, ..env_in(dir, core) }
}

fn spec(kind: RunKind, base: &Arm, cand: &Arm, sessions: u32, targets: &[&str], null_ref: Option<&str>) -> RunSpec {
    RunSpec {
        kind,
        workload: "read".into(),
        base: base.clone(),
        cand: cand.clone(),
        sessions,
        targets: targets.iter().map(|s| s.to_string()).collect(),
        rule: None,
        null_ref: null_ref.map(String::from),
    }
}

fn record(env: &RunEnv, id: &str) -> Value {
    ledger::read_all(&env.ledger).unwrap().into_iter().find(|r| r["id"] == id).expect("record")
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
}

/// A null record (10 tight sessions on g/a, g/b) for `base_sha`. `age_days` = None appends it
/// normally; Some(d) writes it as the first ledger line with `at` d days ago.
fn synthetic_null(env: &RunEnv, base_sha: &str, age_days: Option<u64>) -> String {
    let sessions: Vec<Value> = (0..10)
        .map(|i| {
            let j = 1.0 + (i as f64 - 4.5) * 0.0002;
            json!({"rejection": null, "cases": {"g/a": {"ratio": j}, "g/b": {"ratio": 2.0 - j}}})
        })
        .collect();
    let body = json!({
        "status": "OK", "workload": "read", "test_mode": true,
        "base": {"sha256": base_sha}, "cand": {"sha256": base_sha},
        "cases": ["g/a", "g/b"], "sessions": sessions,
    });
    match age_days {
        None => ledger::append(&env.ledger, "null", body).unwrap(),
        Some(d) => {
            let mut rec = body.as_object().unwrap().clone();
            rec.insert("id".into(), json!("R2-000001"));
            rec.insert("kind".into(), json!("null"));
            rec.insert("at".into(), json!(ledger::iso_utc(now() - d * 86_400)));
            std::fs::write(&env.ledger, format!("{}\n", Value::Object(rec))).unwrap();
            "R2-000001".into()
        }
    }
}

fn valid_sessions(r: &Value) -> usize {
    r["sessions"].as_array().unwrap().iter().filter(|s| s["rejection"].is_null()).count()
}

#[test]
fn null_on_identical_is_tied() {
    let d = tmpdir("null");
    let (b, c) = (arm(&d, "a.sh", "base", &[]), arm(&d, "b.sh", "cand", &[]));
    assert_eq!(b.sha256, c.sha256);
    let env = env_in(&d, 2);
    let id = run(&spec(RunKind::Null, &b, &c, 6, &[], None), &env).unwrap();
    let r = record(&env, &id);
    assert_eq!(r["kind"], "null");
    assert_eq!(r["status"], "OK", "{r}");
    assert_eq!(r["test_mode"], true);
    assert_eq!(r["core"], 2);
    assert!(r["base"]["rev"].is_null() && r["cand"]["rev"].is_null());
    assert_eq!(r["datasets_root"], DATASETS_ROOT);
    assert_eq!(r["sessions_completed"], 6);
    assert_eq!(valid_sessions(&r), 6);
    let w = &r["intervals"]["workload"];
    assert!(w["lo"].as_f64().unwrap() <= 1.0 && w["hi"].as_f64().unwrap() >= 1.0, "{w}");
    assert!(r["b_workload"].as_f64().unwrap() < 0.01, "{}", r["b_workload"]);
    assert!(r["intervals"]["cases"]["g/a"]["point"].is_number());
    // per-session per-case ratios are stored so nullband can recompute any target set
    let s0 = &r["sessions"][0];
    assert!(s0["cases"]["g/b"]["ratio"].is_number() && s0["cases"]["g/b"]["base_ns"].is_number());
    assert!(s0["perf"].is_object() && s0["order"].as_array().unwrap().len() == 2);
    let (bt, bw) = null_band(&r, "read", &["g/a".to_string()]).unwrap();
    assert!(bt.unwrap() < 0.01 && bw < 0.01);
}

#[test]
fn slow_arm_loses() {
    let d = tmpdir("slow");
    let b = arm(&d, "a.sh", "base", &[]);
    let c = arm(&d, "b.sh", "cand", &[("FAKE_FACTOR", "1.2")]);
    let env = env_in(&d, 3);
    let null = synthetic_null(&env, &b.sha256, None);
    let id = run(&spec(RunKind::Calibration, &b, &c, 5, &[], Some(&null)), &env).unwrap();
    let r = record(&env, &id);
    assert_eq!(r["status"], "OK", "{r}");
    assert_eq!(r["verdicts"]["workload"]["verdict"], "LOST");
    assert_eq!(r["verdict"], "LOST");
    assert_eq!(r["T"], 0.05);
}

#[test]
fn fast_arm_wins_on_targets() {
    let d = tmpdir("fast");
    let b = arm(&d, "a.sh", "base", &[]);
    let b2 = arm(&d, "a2.sh", "base", &[]);
    let c = arm(&d, "b.sh", "cand", &[("FAKE_FACTOR", "0.8"), ("FAKE_FACTOR_CASES", "g/a")]);
    let env = env_in(&d, 4);
    // a real null run feeds the confirmation
    let null = run(&spec(RunKind::Null, &b, &b2, 5, &[], None), &env).unwrap();
    let id = run(&spec(RunKind::Confirm, &b, &c, 5, &["g/a"], Some(&null)), &env).unwrap();
    let r = record(&env, &id);
    assert_eq!(r["status"], "OK", "{r}");
    assert_eq!(r["verdicts"]["targets"]["verdict"], "WON");
    assert_eq!(r["verdict"], "WON");
    let wp = r["verdicts"]["workload"]["interval"]["point"].as_f64().unwrap();
    assert!((wp - 0.8f64.sqrt()).abs() < 0.01, "workload point {wp}");
    assert!(r["b_targets"].is_number() && r["b_workload"].is_number());
    assert_eq!(r["null_ref"], null.as_str());
    assert!(r.get("provisional").is_none());
}

#[test]
fn missing_case_rejects() {
    let d = tmpdir("missing");
    let b = arm(&d, "a.sh", "base", &[]);
    let c = arm(&d, "b.sh", "cand", &[("FAKE_CASES", "g/a")]);
    let env = env_in(&d, 6);
    let null = synthetic_null(&env, &b.sha256, None);
    let id = run(&spec(RunKind::Calibration, &b, &c, 5, &[], Some(&null)), &env).unwrap();
    let r = record(&env, &id);
    assert_eq!(r["status"], "NOISE_GATE_FAILED", "{r}");
    let s = r["sessions"].as_array().unwrap();
    assert_eq!(s.len(), 4);
    for x in s {
        assert_eq!(x["rejection"], json!({"MissingCase": "g/b"}), "{x}");
    }
    assert!(r.get("verdict").map_or(true, |v| v.is_null()));
}

#[test]
fn process_failure_rejects() {
    let d = tmpdir("fail");
    let b = arm(&d, "a.sh", "base", &[]);
    let c = arm(&d, "b.sh", "cand", &[("FAKE_EXIT", "3")]);
    let env = env_in(&d, 7);
    let null = synthetic_null(&env, &b.sha256, None);
    let id = run(&spec(RunKind::Calibration, &b, &c, 5, &[], Some(&null)), &env).unwrap();
    let r = record(&env, &id);
    assert_eq!(r["status"], "NOISE_GATE_FAILED", "{r}");
    let s = r["sessions"].as_array().unwrap();
    assert_eq!(s.len(), 4);
    for x in s {
        let msg = x["rejection"]["ProcessFailed"].as_str().unwrap_or_else(|| panic!("{x}"));
        assert!(msg.contains("failing with exit 3"), "{msg}");
    }
    assert_eq!(r["sessions_completed"], 0);
}

#[test]
fn ctx_switch_gate_rejects() {
    let d = tmpdir("ctxsw");
    let b = arm(&d, "a.sh", "base", &[]);
    let c = arm(&d, "b.sh", "cand", &[("FAKE_FACTOR", "0.9")]);
    let mut env = env_in(&d, 11);
    env.probe = Arc::new(CoreProbe { core: 11, calls: AtomicU64::new(0), extra_switches: Some(1_000) });
    let null = synthetic_null(&env, &b.sha256, None);
    let id = run(&spec(RunKind::Screen, &b, &c, 2, &[], Some(&null)), &env).unwrap();
    let r = record(&env, &id);
    assert_eq!(r["status"], "NOISE_GATE_FAILED", "{r}");
    for s in r["sessions"].as_array().unwrap() {
        assert!(s["rejection"]["CtxSwitches"].as_f64().unwrap() > 50.0, "{s}");
        let p = &s["procs"]["base"];
        assert_eq!(p["nivcsw_gated"].as_u64().unwrap(), p["nivcsw"].as_u64().unwrap() + 1_000);
    }
}

#[test]
fn same_sha_refused() {
    let d = tmpdir("same");
    let (b, c) = (arm(&d, "a.sh", "base", &[]), arm(&d, "b.sh", "cand", &[]));
    let env = env_in(&d, 8);
    let null = synthetic_null(&env, &b.sha256, None);
    let id = run(&spec(RunKind::Confirm, &b, &c, 5, &[], Some(&null)), &env).unwrap();
    let r = record(&env, &id);
    assert_eq!(r["status"], "refused");
    assert_eq!(r["sessions"].as_array().unwrap().len(), 0);
    assert!(r["reason"].as_str().unwrap().contains("sha256"), "{r}");
    assert!(!env.scratch.join("s0").exists(), "no session may run");
}

#[test]
fn stale_null_refused() {
    let d = tmpdir("stale");
    let b = arm(&d, "a.sh", "base", &[]);
    let c = arm(&d, "b.sh", "cand", &[("FAKE_FACTOR", "0.9")]);
    let env = env_in(&d, 8);
    let null = synthetic_null(&env, &b.sha256, Some(8));
    let id = run(&spec(RunKind::Confirm, &b, &c, 5, &[], Some(&null)), &env).unwrap();
    let r = record(&env, &id);
    assert_eq!(r["status"], "refused", "{r}");
    assert!(r["reason"].as_str().unwrap().contains("days"), "{r}");
    assert_eq!(r["sessions"].as_array().unwrap().len(), 0);
}

#[test]
fn null_on_other_base_refused() {
    let d = tmpdir("otherbase");
    let b = arm(&d, "a.sh", "base", &[]);
    let c = arm(&d, "b.sh", "cand", &[("FAKE_FACTOR", "0.9")]);
    let env = env_in(&d, 8);
    let null = synthetic_null(&env, "0000", None);
    let id = run(&spec(RunKind::Screen, &b, &c, 5, &[], Some(&null)), &env).unwrap();
    assert_eq!(record(&env, &id)["status"], "refused");
    let id = run(&spec(RunKind::Screen, &b, &c, 5, &[], None), &env).unwrap();
    assert_eq!(record(&env, &id)["status"], "refused");
}

#[test]
fn regression_flagged() {
    let d = tmpdir("regr");
    let b = arm(&d, "a.sh", "base", &[]);
    let c = arm(&d, "b.sh", "cand", &[("FAKE_FACTOR", "1.3"), ("FAKE_FACTOR_CASES", "g/b")]);
    let env = env_in(&d, 9);
    let null = synthetic_null(&env, &b.sha256, None);
    let id = run(&spec(RunKind::Crosscheck, &b, &c, 5, &[], Some(&null)), &env).unwrap();
    let r = record(&env, &id);
    assert_eq!(r["status"], "OK", "{r}");
    assert_eq!(r["results"]["g/b"]["REGRESSION"], true);
    assert_eq!(r["results"]["g/b"]["perf_gate_10pct"], true);
    assert_eq!(r["results"]["g/a"]["REGRESSION"], false);
    assert_eq!(r["results"]["g/a"]["perf_gate_10pct"], false);
    assert!(r.get("verdict").is_none() && r.get("verdicts").is_none(), "a cross-check issues no verdict: {r}");
    assert!(r["workload_interval"]["lo"].is_number());
    let out = Command::new(env!("CARGO_BIN_EXE_r2"))
        .args(["report", &id])
        .env("R2_LEDGER", &env.ledger)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let md = String::from_utf8_lossy(&out.stdout);
    assert!(md.contains("crosscheck") && md.contains("g/b") && md.contains("REGRESSION"), "{md}");
    assert!(md.contains("protocol deviation"), "{md}");
}

#[test]
fn screen_is_provisional() {
    let d = tmpdir("screen");
    let b = arm(&d, "a.sh", "base", &[]);
    let c = arm(&d, "b.sh", "cand", &[("FAKE_FACTOR", "0.98")]);
    let env = env_in(&d, 10);
    let null = synthetic_null(&env, &b.sha256, None);
    let id = run(&spec(RunKind::Screen, &b, &c, 3, &[], Some(&null)), &env).unwrap();
    let r = record(&env, &id);
    assert_eq!(r["provisional"], true);
    assert_eq!(r["kind"], "screen");
}

#[test]
fn nullband_rejects_wrong_workload() {
    let d = tmpdir("nb");
    let env = env_in(&d, 8);
    let id = synthetic_null(&env, "ab", None);
    let r = record(&env, &id);
    assert!(null_band(&r, "decode", &[]).is_err());
    assert!(null_band(&r, "read", &["g/zzz".to_string()]).is_err());
    let (bt, bw) = null_band(&r, "read", &[]).unwrap();
    assert!(bt.is_none() && bw < 0.01);
}

fn cli_env(cmd: &mut Command, d: &Path) {
    cmd.env("R2_TEST_MODE", "1")
        .env("R2_LEDGER", d.join("ledger.jsonl"))
        .env("R2_LOCK", d.join("r2.lock"))
        .env("R2_SCRATCH", d.join("scratch"));
}

#[test]
fn abort_records() {
    let d = tmpdir("abort");
    let b = arm(&d, "a.sh", "base", &[("FAKE_SLEEP", "1")]);
    let c = arm(&d, "b.sh", "cand", &[("FAKE_SLEEP", "1")]);
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_r2"));
    cmd.args(["run", "--kind", "null", "--workload", "read", "--sessions", "20", "--base"])
        .arg(&b.bin)
        .arg("--cand")
        .arg(&c.bin);
    cli_env(&mut cmd, &d);
    let mut child = cmd.spawn().unwrap();
    let t = Instant::now();
    // progress.json is rewritten after every session
    let completed = || -> u64 {
        std::fs::read_to_string(d.join("scratch/progress.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| v["sessions_completed"].as_u64())
            .unwrap_or(0)
    };
    while completed() < 1 {
        assert!(t.elapsed() < Duration::from_secs(60), "first session never completed");
        assert!(child.try_wait().unwrap().is_none(), "r2 exited early");
        std::thread::sleep(Duration::from_millis(50));
    }
    unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
    let st = child.wait().unwrap();
    assert!(!st.success());
    assert!(t.elapsed() < Duration::from_secs(30));
    let recs = ledger::read_all(&d.join("ledger.jsonl")).unwrap();
    let last = recs.last().expect("an aborted record");
    assert_eq!(last["status"], "aborted", "{last}");
    assert!(last["sessions_completed"].as_u64().unwrap() >= 1);
    assert_eq!(last["test_mode"], true);
    let free = Command::new("flock").arg("-n").arg(d.join("r2.lock")).arg("true").status().unwrap();
    assert!(free.success(), "lock still held");
}

#[test]
fn test_mode_refuses_real_ledger() {
    let before = std::fs::read(REAL_LEDGER).ok();
    let d = tmpdir("real");
    let (b, c) = (arm(&d, "a.sh", "base", &[]), arm(&d, "b.sh", "cand", &[]));
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_r2"));
    cmd.args(["run", "--kind", "null", "--workload", "read", "--sessions", "1", "--base"])
        .arg(&b.bin)
        .arg("--cand")
        .arg(&c.bin);
    cli_env(&mut cmd, &d);
    cmd.env("R2_LEDGER", REAL_LEDGER);
    let out = cmd.output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("test mode"), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(std::fs::read(REAL_LEDGER).ok(), before, "the real ledger changed");
    assert!(!d.join("scratch").exists());
}

#[test]
fn cli_nullband_prints_t() {
    let d = tmpdir("clinb");
    let env = env_in(&d, 8);
    let id = synthetic_null(&env, "ab", None);
    let tf = d.join("targets.json");
    std::fs::write(&tf, r#"["g/a"]"#).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_r2"))
        .args(["nullband", "--null", &id, "--workload", "read", "--targets"])
        .arg(&tf)
        .env("R2_LEDGER", &env.ledger)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v["b_targets"].is_number() && v["b_workload"].is_number());
    assert_eq!(v["T"], 0.05);
}

#[test]
fn manifest_mismatch_refused_outside_test_mode() {
    let d = tmpdir("manifest");
    let b = arm(&d, "a.sh", "base", &[]);
    let c = arm(&d, "b.sh", "cand", &[("FAKE_FACTOR", "0.9")]);
    let env = real_env_in(&d, 8);
    // missing manifests
    let id = run(&spec(RunKind::Screen, &b, &c, 2, &[], Some("R2-000099")), &env).unwrap();
    let r = record(&env, &id);
    assert_eq!(r["status"], "refused");
    assert!(r["reason"].as_str().unwrap().contains("manifest"), "{r}");
    assert!(r.get("test_mode").is_none());
    // cand manifest names another binary
    write_manifest(&b, None);
    write_manifest(&c, Some("00ff"));
    let id = run(&spec(RunKind::Screen, &b, &c, 2, &[], Some("R2-000099")), &env).unwrap();
    let r = record(&env, &id);
    assert_eq!(r["status"], "refused");
    let reason = r["reason"].as_str().unwrap();
    assert!(reason.contains("manifest") && reason.contains("00ff"), "{reason}");
    assert!(!env.scratch.join("s0").exists());
}

#[test]
fn manifest_recorded() {
    let d = tmpdir("manifestok");
    let (b, c) = (arm(&d, "a.sh", "base", &[]), arm(&d, "b.sh", "cand", &[]));
    write_manifest(&b, None);
    write_manifest(&c, None);
    let env = env_in(&d, 12);
    let id = run(&spec(RunKind::Null, &b, &c, 1, &[], None), &env).unwrap();
    let r = record(&env, &id);
    assert_eq!(r["status"], "OK", "{r}");
    assert_eq!(r["base"]["rev"]["cqlite_commit"], "7f9abe3663254724439dbe71aff77bb93faa1c2d");
    assert_eq!(r["cand"]["rev"]["sha256"], c.sha256.as_str());
}

#[test]
fn datasets_root_required_outside_test_mode() {
    let d = tmpdir("datasets");
    let b = arm(&d, "a.sh", "base", &[]);
    let c = arm(&d, "b.sh", "cand", &[("FAKE_FACTOR", "0.9")]);
    write_manifest(&b, None);
    write_manifest(&c, None);
    for root in [None, Some("/tmp/elsewhere")] {
        let env = RunEnv { datasets_root: root.map(String::from), ..real_env_in(&d, 8) };
        let id = run(&spec(RunKind::Screen, &b, &c, 2, &[], Some("R2-000099")), &env).unwrap();
        let r = record(&env, &id);
        assert_eq!(r["status"], "refused");
        assert!(r["reason"].as_str().unwrap().contains("CQLITE_DATASETS_ROOT"), "{r}");
    }
}

#[test]
fn null_case_set_mismatch_issues_no_verdict() {
    let d = tmpdir("nullmis");
    let b = arm(&d, "a.sh", "base", &[("FAKE_CASES", "g/a,g/b,g/c")]);
    let c = arm(&d, "b.sh", "cand", &[("FAKE_CASES", "g/a,g/b,g/c"), ("FAKE_FACTOR", "0.8")]);
    let env = env_in(&d, 13);
    let null = synthetic_null(&env, &b.sha256, None); // cases g/a, g/b
    let id = run(&spec(RunKind::Confirm, &b, &c, 2, &[], Some(&null)), &env).unwrap();
    let r = record(&env, &id);
    assert_eq!(r["status"], "NULL_MISMATCH", "{r}");
    assert!(r["reason"].as_str().unwrap().contains("null case set differs"));
    assert_eq!(r["null_cases_match"], false);
    for k in ["verdict", "verdicts", "results", "T"] {
        assert!(r.get(k).is_none(), "{k} present: {r}");
    }
    assert_eq!(valid_sessions(&r), 2);
    let out = Command::new(env!("CARGO_BIN_EXE_r2")).args(["report", &id]).env("R2_LEDGER", &env.ledger).output().unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("null case set differs"));
}

#[test]
fn zero_median_rejects_session() {
    let d = tmpdir("zero");
    let b = arm(&d, "a.sh", "base", &[]);
    let c = arm(&d, "b.sh", "cand", &[("FAKE_ZERO_CASE", "g/b")]);
    let env = env_in(&d, 14);
    let null = synthetic_null(&env, &b.sha256, None);
    let id = run(&spec(RunKind::Calibration, &b, &c, 2, &[], Some(&null)), &env).unwrap();
    let r = record(&env, &id);
    assert_eq!(r["status"], "NOISE_GATE_FAILED", "{r}");
    let s = r["sessions"].as_array().unwrap();
    assert_eq!(s.len(), 4);
    for x in s {
        assert_eq!(x["rejection"], json!({"ProcessFailed": "non-finite median for g/b"}), "{x}");
    }
}

#[test]
fn library_refuses_test_mode_on_real_ledger() {
    let before = std::fs::read(REAL_LEDGER).ok();
    let d = tmpdir("libreal");
    let (b, c) = (arm(&d, "a.sh", "base", &[]), arm(&d, "b.sh", "cand", &[]));
    let env = RunEnv { ledger: PathBuf::from(REAL_LEDGER), ..env_in(&d, 8) };
    let e = run(&spec(RunKind::Null, &b, &c, 1, &[], None), &env).unwrap_err();
    assert!(e.contains("test mode"), "{e}");
    assert_eq!(std::fs::read(REAL_LEDGER).ok(), before);
    assert!(!d.join("r2.lock").exists() && !env.scratch.exists());
}

#[test]
fn ledger_append_subcommand() {
    let d = tmpdir("lappend");
    let (led, body) = (d.join("l.jsonl"), d.join("b.json"));
    std::fs::write(&body, r#"{"rev":"abc","mode":"gate"}"#).unwrap();
    let run = |k: &str| {
        Command::new(env!("CARGO_BIN_EXE_r2"))
            .args(["ledger-append", "--kind", k, "--body-file"]).arg(&body)
            .env("R2_LEDGER", &led).output().unwrap()
    };
    let o = run("tests");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(String::from_utf8_lossy(&o.stdout).trim(), "R2-000001");
    assert_eq!(String::from_utf8_lossy(&run("tests").stdout).trim(), "R2-000002");
    let r = ledger::read_all(&led).unwrap();
    assert_eq!((r[0]["kind"].as_str(), r[0]["rev"].as_str()), (Some("tests"), Some("abc")));
    let before = std::fs::read(&led).unwrap();
    let o = run("null");
    assert!(!o.status.success() && String::from_utf8_lossy(&o.stderr).contains("only --kind tests"));
    assert_eq!(std::fs::read(&led).unwrap(), before, "refused kind must write nothing");
    std::fs::write(&body, "[1]").unwrap();
    assert!(!run("tests").status.success());
}
