//! Hardware tests. They share one core-pinned measurement path and the host, so they must never
//! overlap: every test takes `SERIAL` (equivalent to `--test-threads=1`).
//! Sizes: MSX_SESSIONS=2 MSX_BLOCKS=30 (set below), recorded in the result.

use msx_harness::build::{a_path, Rung};
use msx_harness::counters::Counters;
use msx_harness::loader::Lib;
use msx_harness::measure::{load_private_copy, run_protocol, Contestant, Protocol, Split};
use std::sync::Mutex;

static SERIAL: Mutex<()> = Mutex::new(());

fn work() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../work")
}

#[test]
fn counters_open_and_count() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let c = Counters::open().expect("perf_event_open refused (stop-gate 3)");
    let a = c.read();
    let mut x = 1u64;
    for i in 0..2_000_000u64 {
        x = std::hint::black_box(x.wrapping_mul(31).wrapping_add(i));
    }
    std::hint::black_box(x);
    let b = c.read();
    assert!(b.cycles > a.cycles, "{a:?} {b:?}");
    assert!(b.instructions > a.instructions);
    assert_eq!(b.time_enabled - a.time_enabled, b.time_running - a.time_running, "multiplexed");
}

#[test]
fn r1_against_its_copy() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("MSX_SESSIONS", "2");
    std::env::set_var("MSX_BLOCKS", "30");
    std::env::set_var("MSX_EXE", env!("CARGO_BIN_EXE_msx"));
    let tmp = work().join("tmp");
    std::fs::create_dir_all(&tmp).unwrap();
    let copy = tmp.join(format!("r1copy-{}.so", std::process::id()));
    std::fs::copy(a_path(Rung::R1), &copy).unwrap();
    let cs = vec![
        Contestant { name: "r1".into(), so: a_path(Rung::R1), symbol: "msx_vint".into(), is_chain: false },
        Contestant { name: "r1copy".into(), so: copy.clone(), symbol: "msx_vint".into(), is_chain: false },
    ];
    for split in [Split::Search, Split::Holdout] {
        let res = run_protocol("vint", &cs, Protocol::Search, split).expect("run_protocol");
        eprintln!("{split:?}: {}", serde_json::to_string_pretty(&res).unwrap());
        assert_eq!((res.sessions_used, res.blocks_used), (2, 30));
        assert!(res.size_override);
        assert_eq!(res.sessions.len(), 2);
        for s in &res.sessions {
            assert!(s.valid_blocks >= 10, "{s:?}");
        }
        let a = res.medians["r1"].instructions_per_op;
        let b = res.medians["r1copy"].instructions_per_op;
        assert!(((a - b) / a).abs() < 1e-4, "instructions/op {a} vs {b}");
        let p = res.pair("r1copy", "r1").expect("pair").interval.expect("interval");
        assert!((0.8..=1.25).contains(&p.point), "{p:?}");
    }
    let _ = std::fs::remove_file(&copy);
}

#[test]
fn contestant_with_reference_path_gets_its_own_mapping() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let reference = Lib::open(&a_path(Rung::R1)).unwrap();
    let again = Lib::open(&a_path(Rung::R1)).unwrap();
    let private = load_private_copy(&a_path(Rung::R1), "test").unwrap();
    let r = reference.sym("msx_vint").unwrap() as usize;
    assert_eq!(again.sym("msx_vint").unwrap() as usize, r, "plain dlopen of the same path is deduplicated");
    assert_ne!(private.sym("msx_vint").unwrap() as usize, r, "private copy must map at its own address");
}

// ---- candidate failures during measurement (fix wave I1) ----

mod support;
use msx_harness::measure::{fresh_nonce, run_protocol_with, session_seed, ContestantFailure};

fn small_sizes() {
    std::env::set_var("MSX_SESSIONS", "2");
    std::env::set_var("MSX_BLOCKS", "30");
    std::env::set_var("MSX_EXE", env!("CARGO_BIN_EXE_msx"));
}

fn pair_with(name: &str, fixture: &str) -> Vec<Contestant> {
    let so = support::build_fixture_candidate(fixture).unwrap_or_else(|e| panic!("{fixture}: {} {}", e.stage, e.stderr));
    vec![
        Contestant { name: "A.r1".into(), so: a_path(Rung::R1), symbol: "msx_vint".into(), is_chain: false },
        Contestant { name: name.into(), so, symbol: "msx_vint".into(), is_chain: false },
    ]
}

fn failure_of(cs: &[Contestant], secs: u64) -> Result<ContestantFailure, String> {
    let e = match run_protocol_with("vint", cs, Protocol::Search, Split::Search, std::time::Duration::from_secs(secs)) {
        Ok(_) => panic!("the run must fail"),
        Err(e) => e,
    };
    ContestantFailure::parse(&e).ok_or(e)
}

#[test]
fn a_diverging_candidate_is_attributed() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    small_sizes();
    let f = failure_of(&pair_with("cand", "cliff_len.s"), 120).expect("a structured failure");
    assert_eq!((f.contestant.as_str(), f.verdict.as_str()), ("cand", "INCORRECT"), "{f:?}");
    assert!(f.detail.contains("diverged"), "{f:?}");
}

#[test]
fn a_crashing_candidate_is_attributed() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    small_sizes();
    let f = failure_of(&pair_with("B.asm", "vint_crash.s"), 120).expect("a structured failure");
    assert_eq!((f.contestant.as_str(), f.verdict.as_str()), ("B.asm", "OOB_OR_CRASH"), "{f:?}");
}

#[test]
fn a_hanging_candidate_times_out() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    small_sizes();
    let f = failure_of(&pair_with("cand", "vint_hang.s"), 15).expect("a structured failure");
    assert_eq!((f.contestant.as_str(), f.verdict.as_str()), ("cand", "TIMEOUT"), "{f:?}");
}

#[test]
fn a_diverging_a_contestant_stays_a_tool_error() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    small_sizes();
    let e = failure_of(&pair_with("A.r3", "cliff_len.s"), 120).expect_err("not a candidate failure");
    assert!(e.contains("A.r3"), "{e}");
}

#[test]
fn every_run_uses_its_own_session_seeds() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    small_sizes();
    let cs = vec![Contestant { name: "r1".into(), so: a_path(Rung::R1), symbol: "msx_vint".into(), is_chain: false }];
    let run = || run_protocol_with("vint", &cs, Protocol::Search, Split::Search, std::time::Duration::from_secs(300)).expect("run_protocol");
    let (a, b) = (run(), run());
    assert_ne!(a.nonce, b.nonce);
    assert_ne!(a.sessions[0].seed, b.sessions[0].seed);
    let n = u64::from_str_radix(a.nonce.trim_start_matches("0x"), 16).unwrap();
    for s in &a.sessions {
        assert_eq!(s.seed, session_seed(s.session, s.reruns_before, n));
    }
    let _ = fresh_nonce();
}
