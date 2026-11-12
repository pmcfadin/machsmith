use msx_harness::ledger::{append, budget, read_all};
use serde_json::json;
use std::sync::Mutex;

// MSX_LEDGER is process-global: serialize the tests that set it.
static ENV: Mutex<()> = Mutex::new(());

fn tmp_ledger(tag: &str) -> std::path::PathBuf {
    let work = std::env::var("MSX_WORK").unwrap_or_else(|_| format!("{}/../../work", env!("CARGO_MANIFEST_DIR")));
    let dir = std::path::PathBuf::from(work).join("tmp").join(format!("ledger-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("ledger.jsonl")
}

#[test]
fn appends_get_sequential_ids_and_common_fields() {
    let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let path = tmp_ledger("seq");
    std::env::set_var("MSX_LEDGER", &path);
    assert!(read_all().is_empty());
    let a = append(json!({"kind": "selftest", "kernel": "vint"})).unwrap();
    let b = append(json!({"kind": "selftest", "kernel": "vint"})).unwrap();
    assert_eq!((a.as_str(), b.as_str()), ("L000001", "L000002"));
    let all = read_all();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0]["id"], "L000001");
    assert_eq!(all[1]["id"], "L000002");
    for r in &all {
        for f in ["id", "ts", "git_head", "git_dirty", "host", "tool_sha256", "a_sha256"] {
            assert!(r.get(f).is_some(), "missing common field {f}: {r}");
        }
        assert!(r["ts"].as_str().unwrap().ends_with('Z'));
        assert!(r["git_dirty"].is_boolean());
        for f in ["cpu_model", "kernel_release", "governor", "load_avg_1m"] {
            assert!(r["host"].get(f).is_some(), "host.{f} missing");
        }
        assert_eq!(r["kernel"], "vint");
    }
    // the file really is one JSON object per line
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(text.lines().count(), 2);
    std::env::remove_var("MSX_LEDGER");
}

#[test]
fn caller_cannot_forge_common_fields() {
    let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let path = tmp_ledger("forge");
    std::env::set_var("MSX_LEDGER", &path);
    append(json!({"kind": "selftest", "id": "L999999", "tool_sha256": "x"})).unwrap();
    let r = &read_all()[0];
    assert_eq!(r["id"], "L000001");
    assert_ne!(r["tool_sha256"], "x");
    std::env::remove_var("MSX_LEDGER");
}

#[test]
fn budget_counts_submissions_and_counted_attempts() {
    let mut recs = Vec::new();
    let attempt = |k: &str, arm: &str, counted: Option<u32>, verdict: &str| {
        json!({"kind": "attempt", "kernel": k, "arm": arm, "counted_attempt": counted, "verdict": verdict})
    };
    for _ in 0..4 {
        recs.push(attempt("vint", "asm", None, "LINT_FAILED"));
    }
    for i in 1..=3 {
        recs.push(attempt("vint", "asm", Some(i), "INCORRECT"));
    }
    for i in 4..=5 {
        recs.push(attempt("vint", "asm", Some(i), "PASS"));
    }
    recs.push(attempt("vint", "asm", None, "NOISE_GATE_FAILED"));
    // refused records and other (kernel, arm) pairs do not count
    recs.push(json!({"kind": "refused", "kernel": "vint", "arm": "asm", "counted_attempt": null}));
    recs.push(json!({"kind": "null", "kernel": "vint"}));
    recs.push(attempt("vint", "rust", Some(1), "PASS"));
    recs.push(attempt("index_framing", "asm", Some(1), "PASS"));
    let b = budget(&recs, "vint", "asm");
    assert_eq!((b.submissions, b.counted), (10, 5));
    let r = budget(&recs, "vint", "rust");
    assert_eq!((r.submissions, r.counted), (1, 1));
}
