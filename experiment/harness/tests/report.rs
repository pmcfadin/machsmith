//! `msx report`: reads the ledger and `results/` and prints Markdown. Synthetic fixtures only.

use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::{Command, Output};

fn scratch(tag: &str) -> PathBuf {
    let base = std::env::var("MSX_WORK").unwrap_or_else(|_| format!("{}/../../work", env!("CARGO_MANIFEST_DIR")));
    let dir = PathBuf::from(base).join("tmp").join(format!("report-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("experiment/results/final")).unwrap();
    dir
}

fn iv(p: f64, lo: f64, hi: f64) -> Value {
    json!({ "point": p, "lo": lo, "hi": hi })
}

fn common(id: &str, kind: &str) -> Value {
    json!({
        "id": id, "kind": kind, "ts": "2026-11-12T05:00:00Z", "git_head": "abc", "git_dirty": true,
        "tool_sha256": "aa".repeat(32), "a_sha256": { "r1": "11", "r2": "22", "r3": "33", "r3copy": "33" },
        "host": { "cpu_model": "Test CPU", "kernel_release": "9.9", "governor": "powersave" },
    })
}

fn attempt(id: &str, submission: u32, counted: Value, verdict: &str, measure: Value) -> Value {
    let mut r = common(id, "attempt");
    r["kernel"] = "vint".into();
    r["arm"] = "asm".into();
    r["submission"] = submission.into();
    r["counted_attempt"] = counted;
    r["verdict"] = verdict.into();
    r["measure"] = measure;
    r
}

fn fixture(tag: &str) -> PathBuf {
    let root = scratch(tag);
    let results = root.join("experiment/results");
    let search = json!({ "pairs": [ { "cand": "cand", "base": "A.r3", "interval": iv(0.833, 0.832, 0.834) } ] });
    let mut final_rec = common("L000042", "final");
    final_rec["kernel"] = "vint".into();
    final_rec["status"] = "OK".into();
    let verdicts = json!([
        { "cand": "B.asm", "base": "A.r3", "interval": iv(0.912, 0.901, 0.923), "verdict": "WON" },
        { "cand": "A.r3", "base": "A.r2", "interval": iv(0.943, 0.942, 0.944), "verdict": "WON" },
        { "cand": "B.asm", "base": "A.r2", "interval": iv(0.8051, 0.8047, 0.8055), "verdict": "WON" },
        { "cand": "B.asm", "base": "A.r1", "interval": iv(0.80512, 0.80511, 0.80514), "verdict": "WON" },
    ]);
    let med = |c: f64| json!({ "cycles_per_op": c, "instructions_per_op": 40.0, "branch_misses_per_op": 0.03, "ns_per_op": 2.0 });
    let fin = json!({
        "id": "L000042", "kernel": "vint", "kind": "final", "status": "OK", "band": 0.0004, "threshold": 0.05,
        "arms": {
            "asm": { "status": "ENTERED", "attempt": 2, "candidate_sha256": "ab".repeat(32), "check": { "cases_run": 1234, "verdict": "PASS" } },
            "rust": { "status": "NO_ENTRY", "reason": "FINAL.md is missing", "attempt": null },
        },
        "verdicts": verdicts,
        "nulls": { "A.r3copy/A.r3": iv(1.0001, 0.9999, 1.0004) },
        "boundary_tax": { "base": "A.inline", "cand": "A.r3", "interval": iv(1.5, 1.49, 1.51) },
        "measure": {
            "split": "holdout", "inputs_repeat": true, "sessions_used": 30, "blocks_used": 200, "core": 5, "reruns": 0,
            "medians": { "A.r3": med(11.0), "B.asm": med(10.0) },
        },
    });
    for (k, v) in fin.as_object().unwrap() {
        final_rec[k] = v.clone();
    }
    let ledger: Vec<Value> = vec![
        attempt("L000040", 1, Value::Null, "LINT_FAILED", Value::Null),
        attempt("L000041", 2, 1.into(), "PASS", search),
        final_rec,
    ];
    let text: String = ledger.iter().map(|r| format!("{r}\n")).collect();
    std::fs::write(results.join("ledger.jsonl"), text).unwrap();

    std::fs::write(results.join("final/vint.json"), serde_json::to_string_pretty(&fin).unwrap()).unwrap();
    root
}

fn report(root: &PathBuf) -> Output {
    Command::new(env!("CARGO_BIN_EXE_msx"))
        .current_dir(root.join("experiment"))
        .arg("report")
        .env("MSX_ROOT", root)
        .env("MSX_WORK", root.join("work"))
        .env("MSX_LEDGER", root.join("experiment/results/ledger.jsonl"))
        .output()
        .unwrap()
}

#[test]
fn report_prints_verdict_row_history_label_and_no_entry() {
    let root = fixture("main");
    let o = report(&root);
    let out = String::from_utf8_lossy(&o.stdout).into_owned();
    assert!(o.status.success(), "{}\n{}", out, String::from_utf8_lossy(&o.stderr));
    assert!(out.contains("| B.asm vs A.r3 | 0.912 | 0.901–0.923 | WON | L000042 |"), "{out}");
    assert!(out.contains("| A.r3 vs A.r2 | 0.943 | 0.942–0.944 | WON | L000042 |"), "{out}");
    // a narrow interval gets the fewest decimals (3 to 5) at which lo and hi differ
    assert!(out.contains("| B.asm vs A.r2 | 0.8051 | 0.8047–0.8055 | WON | L000042 |"), "{out}");
    assert!(out.contains("| B.asm vs A.r1 | 0.80512 | 0.80511–0.80514 | WON | L000042 |"), "{out}");
    // asm entered, rust has no entry: the direct comparison says NO_ENTRY, not ENTERED
    assert!(out.contains("| B.asm vs B.rust | – | – | NO_ENTRY | L000042 |"), "{out}");
    assert!(out.contains("provisional (search phase)"), "{out}");
    assert!(out.contains("NO_ENTRY"), "{out}");
    // a row without a measured verdict for an arm that has no entry says so
    assert!(out.contains("| B.rust vs A.r3 | – | – | NO_ENTRY | L000042 |"), "{out}");
    // attempts history: free failure and counted pass, each with its ledger id
    assert!(out.contains("| 1 | free | LINT_FAILED | LINT_FAILED | L000040 |"), "{out}");
    assert!(out.contains("| 2 | 1 | PASS | 0.833 (0.832–0.834) | L000041 |"), "{out}");
    // ceiling table is absent from the fixture; the report says so instead of failing
    assert!(out.contains("no profile data"), "{out}");
    // the incidents file is missing
    assert!(out.contains("no incidents file"), "{out}");
    // null, boundary tax, counters and provenance
    assert!(out.contains("A.r3copy vs A.r3 (null)"), "{out}");
    assert!(out.contains("boundary tax"), "{out}");
    assert!(out.contains("| B.asm | 10.00 | 40.00 | 0.0300 | 2.000 | L000042 |"), "{out}");
    assert!(out.contains("L000040") && out.contains("L000042"), "{out}");
}

#[test]
fn report_only_reads() {
    let root = fixture("readonly");
    let results = root.join("experiment/results");
    let before = std::fs::read(results.join("ledger.jsonl")).unwrap();
    let o = report(&root);
    assert!(o.status.success());
    assert_eq!(std::fs::read(results.join("ledger.jsonl")).unwrap(), before);
    assert!(!results.join("ledger.lock").exists(), "report must not take the ledger lock");
    assert!(!root.join("work/msx.lock").exists(), "report must not take work/msx.lock");
}

#[test]
fn report_prints_incidents_file_when_present() {
    let root = fixture("incidents");
    std::fs::write(root.join("experiment/results/incidents.md"), "# Incidents\n\nbursty builds\n").unwrap();
    let out = String::from_utf8_lossy(&report(&root).stdout).into_owned();
    assert!(out.contains("bursty builds"), "{out}");
    assert!(!out.contains("no incidents file"), "{out}");
}

#[test]
fn report_warns_when_final_file_differs_from_ledger_and_skips_bad_lines() {
    let root = fixture("warn");
    let l = root.join("experiment/results/ledger.jsonl");
    let mut t = std::fs::read_to_string(&l).unwrap();
    t.push_str("not json\n");
    std::fs::write(&l, t).unwrap();
    let f = root.join("experiment/results/final/vint.json");
    let mut j: Value = serde_json::from_str(&std::fs::read_to_string(&f).unwrap()).unwrap();
    j["band"] = json!(0.5);
    std::fs::write(&f, j.to_string()).unwrap();
    let out = String::from_utf8_lossy(&report(&root).stdout).into_owned();
    assert!(out.contains("1 ledger line(s) are not valid JSON"), "{out}");
    assert!(out.contains("field `band` differs from ledger record L000042"), "{out}");
}
