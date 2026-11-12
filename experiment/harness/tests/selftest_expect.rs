//! The self-test's expectation files: every candidate file has an entry and every entry names a
//! file; plus the pure judging rules of `msx selftest`.

use msx_harness::commands::{judge, parse_expect, selftest_dir, selftest_entries};
use std::collections::BTreeSet;

#[test]
fn every_file_has_an_entry_and_every_entry_a_file() {
    let dir = selftest_dir();
    let entries = selftest_entries(&dir).unwrap_or_else(|e| panic!("{e}"));
    // coverage was checked inside; check it independently here from the file system
    let mut on_disk = BTreeSet::new();
    for sub in std::fs::read_dir(&dir).unwrap().flatten() {
        if !sub.path().is_dir() {
            continue;
        }
        for f in std::fs::read_dir(sub.path()).unwrap().flatten() {
            let n = f.file_name().to_string_lossy().into_owned();
            if n.ends_with(".s") || n.ends_with(".rs") {
                on_disk.insert(format!("{}/{n}", sub.file_name().to_string_lossy()));
            }
        }
    }
    let listed: BTreeSet<String> = entries.iter().map(|e| format!("{}/{}", e.dir, e.file)).collect();
    assert_eq!(on_disk, listed);
    assert!(entries.iter().all(|e| e.path.is_file()));
}

#[test]
fn bti_sparse_find_is_dropped_and_has_no_selftest_files() {
    assert!(!selftest_dir().join("bti_sparse_find").exists());
    assert!(selftest_entries(&selftest_dir()).unwrap().iter().all(|e| e.dir != "bti_sparse_find"));
}

#[test]
fn recorded_expectations_cover_the_task_7_table() {
    let entries = selftest_entries(&selftest_dir()).unwrap();
    let get = |d: &str, f: &str| entries.iter().find(|e| e.dir == d && e.file == f).unwrap_or_else(|| panic!("{d}/{f}")).expect.clone();
    for f in ["syscall.s", "ctor.s", "bigrodata.s", "indirect.s", "calls_out.s", "uses_asm.rs"] {
        assert_eq!(get("vint", f).check, "LINT_FAILED", "{f}");
    }
    assert_eq!(get("vint", "overread1.s").check, "OOB_OR_CRASH");
    assert_eq!(get("vint", "underread1.s").check, "OOB_OR_CRASH");
    assert_eq!(get("vint", "unsafe_overread.rs").check, "OOB_OR_CRASH");
    assert_eq!(get("vint", "wrong_fe.s").check, "INCORRECT");
    assert_eq!(get("vint", "writes_on_error.s").check, "INCORRECT");
    assert_eq!(get("vint", "clobber_rbx.s").check, "ABI_VIOLATION");
    assert_eq!(get("vint", "loop_ff.s").check, "TIMEOUT");
    assert_eq!(get("vint", "control_slow.s").measured.as_deref(), Some("LOST"));
    assert_eq!(get("vint", "control_copy.rs").measured.as_deref(), Some("TIED"));
    assert_eq!(get("vint", "control_ok.s").measured.as_deref(), Some("*"));
    assert_eq!(get("index_framing", "control_slow.s").measured.as_deref(), Some("LOST"));
}

#[test]
fn parse_expect_rejects_bad_values() {
    assert!(parse_expect(r#"{"a.s": "PASS/TIED"}"#).is_ok());
    assert!(parse_expect(r#"{"a.s": "NOPE"}"#).is_err());
    assert!(parse_expect(r#"{"a.s": "PASS/WON?"}"#).is_err());
    assert!(parse_expect(r#"{"a.s": "INCORRECT/TIED"}"#).is_err(), "only PASS can be measured");
    assert!(parse_expect("[]").is_err());
}

#[test]
fn judge_compares_check_and_measured() {
    let e = parse_expect(r#"{"a": "PASS/LOST", "b": "PASS/*", "c": "INCORRECT"}"#).unwrap();
    assert_eq!(judge(&e["a"], "PASS", Some("LOST")), ("PASS/LOST".to_string(), true));
    assert_eq!(judge(&e["a"], "PASS", Some("TIED")), ("PASS/TIED".to_string(), false));
    assert_eq!(judge(&e["a"], "INCORRECT", None), ("INCORRECT".to_string(), false));
    assert_eq!(judge(&e["b"], "PASS", Some("WON")), ("PASS/WON".to_string(), true));
    assert_eq!(judge(&e["b"], "PASS", Some("NOISE_GATE_FAILED")).1, false, "no measurement is not a pass");
    assert_eq!(judge(&e["b"], "PASS", None).1, false);
    assert_eq!(judge(&e["c"], "INCORRECT", None), ("INCORRECT".to_string(), true));
    assert_eq!(judge(&e["c"], "PASS", Some("TIED")).1, false);
}
