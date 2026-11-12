//! Regression tests for the pre-freeze fix wave: toolchain pin, bounded and attributed candidate
//! execution (training runs), chain loops that terminate on a misbehaving kernel, pure helpers.
//! Tests that set process environment variables take `SERIAL`.

use msx_harness::build::{build_rust_candidate, pinned_toolchain, runtime_verdict_of_stage, rustc_command, rustc_version, Rung};
use msx_harness::commands::{check_rerun, mark_noise_gate_failed, refused_env_set, vet_r2, REFUSED_ENV};
use msx_harness::measure::{fresh_nonce, is_candidate_name, session_seed, ContestantFailure, SESSION_SEED_BASE};
use msx_harness::{kernel, RawFn};
use std::path::PathBuf;
use std::sync::Mutex;

mod support;

static SERIAL: Mutex<()> = Mutex::new(());

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

// ---- M1: toolchain pin ----

#[test]
fn rustc_and_cargo_are_pinned_to_the_experiment_toolchain() {
    assert_eq!(pinned_toolchain(), "1.97.1");
    for cmd in [rustc_command(), msx_harness::build::cargo_command()] {
        let pinned = cmd.get_envs().find(|(k, _)| *k == "RUSTUP_TOOLCHAIN").and_then(|(_, v)| v);
        assert_eq!(pinned.map(|v| v.to_string_lossy().into_owned()), Some("1.97.1".to_string()));
    }
    // from a directory outside experiment/, where rust-toolchain.toml does not apply
    let dir = support::scratch("toolchain");
    let out = rustc_command().current_dir(&dir).arg("-V").output().unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("1.97.1"), "{out:?}");
    assert!(rustc_version().contains("1.97.1"), "{}", rustc_version());
}

// ---- I2: the training run of a Rust candidate is bounded and attributed ----

fn train_failure(file: &str) -> msx_harness::build::BuildError {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("MSX_CHECK_TIMEOUT_SECS", "10");
    std::env::set_var("MSX_BIN", env!("CARGO_BIN_EXE_msx"));
    let out = support::scratch(file);
    let started = std::time::Instant::now();
    let e = build_rust_candidate("vint", &fixture(file), Rung::R3, &out).expect_err("the training run must fail");
    assert!(started.elapsed().as_secs() < 120, "took {:?}", started.elapsed());
    e
}

#[test]
fn a_hanging_training_run_times_out() {
    let e = train_failure("rs_hang.rs");
    assert_eq!(e.stage, "train-timeout", "{}", e.stderr);
    assert_eq!(runtime_verdict_of_stage(e.stage), Some("TIMEOUT"));
}

#[test]
fn a_crashing_training_run_is_a_crash_not_a_build_failure() {
    let e = train_failure("rs_crash.rs");
    assert_eq!(e.stage, "train-crash", "{}", e.stderr);
    assert_eq!(runtime_verdict_of_stage(e.stage), Some("OOB_OR_CRASH"));
    // compile and link failures stay free
    assert_eq!(runtime_verdict_of_stage("rustc"), None);
    assert_eq!(runtime_verdict_of_stage("train"), None);
}

// ---- I2: harness chain loops end on a kernel that misbehaves ----

unsafe extern "C" fn vint_overclaims(_p: *const u8, _len: usize, out: *mut u64) -> usize {
    *out = 1;
    1_000_000
}

unsafe extern "C" fn framing_zero_step(_p: *const u8, _len: usize, out: *mut u64) -> u32 {
    *out = 0;
    *out.add(1) = 0;
    0
}

#[test]
fn chain_loops_terminate_on_misbehaving_kernels() {
    let seg = [7u8; 64];
    let v = kernel("vint").unwrap();
    let (ops, _) = unsafe { v.run_chain(vint_overclaims as RawFn, &seg) };
    assert_eq!(ops, 0, "a decoder claiming more than remains ends the chain without counting it");
    let f = kernel("index_framing").unwrap();
    let (ops, _) = unsafe { f.run_chain(framing_zero_step as RawFn, &seg) };
    assert_eq!(ops, 1, "a zero step ends the chain");
}

// ---- I1: contestant failures travel as structured errors ----

#[test]
fn contestant_failures_round_trip_and_only_candidates_count() {
    let f = ContestantFailure { contestant: "B.rust".into(), verdict: "OOB_OR_CRASH".into(), detail: "died | of signal 11".into() };
    assert_eq!(ContestantFailure::parse(&f.to_error()), Some(f));
    assert_eq!(ContestantFailure::parse("NOISE_GATE_FAILED: x"), None);
    assert!(is_candidate_name("cand") && is_candidate_name("B.asm") && is_candidate_name("B.rust.r2"));
    assert!(!is_candidate_name("A.r1") && !is_candidate_name("A.inline"));
}

// ---- M7: nonce ----

#[test]
fn session_seeds_depend_on_a_fresh_nonce() {
    let (a, b) = (fresh_nonce(), fresh_nonce());
    assert_ne!(a, b, "two nonces must differ");
    assert_eq!(session_seed(3, 1, 0), SESSION_SEED_BASE + 3 + 1000);
    assert_ne!(session_seed(0, 0, a), session_seed(0, 0, b));
    assert_ne!(session_seed(0, 0, a), session_seed(1, 0, a));
}

// ---- M2 / M3 / M4: pure rules ----

fn fin(id: &str, status: &str, measured: bool, asm: Option<u64>, rust: Option<u64>) -> serde_json::Value {
    serde_json::json!({
        "id": id, "kind": "final", "kernel": "vint", "status": status,
        "measure": if measured { serde_json::json!({"x": 1}) } else { serde_json::Value::Null },
        "arms": { "asm": { "attempt": asm }, "rust": { "attempt": rust } },
    })
}

#[test]
fn a_final_that_issued_verdicts_is_never_repeated() {
    let l = vec![fin("L000001", "OK", true, Some(3), None)];
    let e = check_rerun(&l, "vint", &[("asm", Some(3)), ("rust", None)], Some("because")).unwrap_err();
    assert!(e.contains("never allowed"), "{e}");
    // another kernel is unaffected
    assert!(check_rerun(&l, "index_framing", &[("asm", Some(3)), ("rust", None)], None).is_ok());
}

#[test]
fn a_rerun_cannot_change_the_pick() {
    let l = vec![fin("L000001", "NOISE_GATE_FAILED", true, Some(3), Some(5))];
    assert!(check_rerun(&l, "vint", &[("asm", Some(3)), ("rust", Some(5))], Some("noise")).is_ok());
    let e = check_rerun(&l, "vint", &[("asm", Some(4)), ("rust", Some(5))], Some("noise")).unwrap_err();
    assert!(e.contains("may not change the pick") && e.contains("asm"), "{e}");
    // a reason is still required after a measured final
    assert!(check_rerun(&l, "vint", &[("asm", Some(3)), ("rust", Some(5))], None).is_err());
    // no second pick after NO_FINALISTS either: an arm that had no entry cannot gain one
    let l = vec![fin("L000002", "NO_FINALISTS", false, None, None)];
    assert!(check_rerun(&l, "vint", &[("asm", Some(1)), ("rust", None)], None).is_err());
    assert!(check_rerun(&l, "vint", &[("asm", None), ("rust", None)], None).is_ok());
    // the first final has nothing to compare with
    assert!(check_rerun(&[], "vint", &[("asm", Some(1)), ("rust", None)], None).is_ok());
}

#[test]
fn noise_gate_failure_sets_the_flag_with_the_status() {
    let mut rec = serde_json::json!({ "status": "NO_FINALISTS", "noise_failure": false });
    mark_noise_gate_failed(&mut rec, "null band 0.0700 > 0.05".into());
    assert_eq!(rec["status"], "NOISE_GATE_FAILED");
    assert_eq!(rec["noise_failure"], true);
    assert_eq!(rec["noise_detail"], "null band 0.0700 > 0.05");
}

#[test]
fn refused_env_names_are_the_four_overrides() {
    assert_eq!(REFUSED_ENV, ["MSX_CHECK_TIMEOUT_SECS", "MSX_EXE", "MSX_BIN", "MSX_SELFTEST_DIR"]);
    assert_eq!(refused_env_set(|n| n == "MSX_BIN" || n == "MSX_LEDGER"), vec!["MSX_BIN"]);
    assert!(refused_env_set(|_| false).is_empty());
}

// ---- I3: B.rust.r2 is vetted like the entry ----

#[test]
fn r2_build_that_fails_lint_or_check_is_not_admitted() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("MSX_CHECK_TIMEOUT_SECS", "10");
    std::env::set_var("MSX_BIN", env!("CARGO_BIN_EXE_msx"));
    let src = fixture("rs_hang.rs");
    // fails the object lint (a syscall instruction)
    let so = support::build_selftest("syscall.s").unwrap();
    let why = vet_r2("vint", &so, &src).unwrap_err();
    assert!(why.starts_with("lint:") && why.contains("syscall"), "{why}");
    // passes the lint but is wrong: rejected by the final check (the first batch finds it)
    let so = support::build_selftest("wrong_fe.s").unwrap();
    let why = vet_r2("vint", &so, &src).unwrap_err();
    assert!(why.starts_with("final check: INCORRECT"), "{why}");
}
