//! Hardware tests. They share one core-pinned measurement path and the host, so they must never
//! overlap: every test takes `SERIAL` (equivalent to `--test-threads=1`).
//! Sizes: MSX_SESSIONS=2 MSX_BLOCKS=30 (set below), recorded in the result.

use msx_harness::build::{a_path, Rung};
use msx_harness::counters::Counters;
use msx_harness::measure::{run_protocol, Contestant, Protocol, Split};
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
