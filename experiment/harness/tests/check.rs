use msx_harness::build::{a_path, Rung};
use msx_harness::check::{check, phase_params, seed_from_source, CheckReport, Phase};
use msx_harness::spec::manifest_segments;
use std::path::PathBuf;

mod support;
use support::{build_selftest, scratch};

fn setup() {
    std::env::set_var("MSX_CHECK_TIMEOUT_SECS", "10");
    std::env::set_var("MSX_BIN", env!("CARGO_BIN_EXE_msx"));
}

fn run(file: &str) -> CheckReport {
    setup();
    let so = build_selftest(file).unwrap_or_else(|e| panic!("{file}: {} {}", e.stage, e.stderr));
    check("vint", &so, Phase::Search, None)
}

fn expect(file: &str, verdict: &str) -> CheckReport {
    let r = run(file);
    assert_eq!(r.verdict, verdict, "{file}: {r:#?}");
    r
}

fn input_len(r: &CheckReport) -> usize {
    r.first_failure.as_ref().expect("no failure recorded").input_hex.len() / 2
}

#[test]
fn control_ok_passes() {
    let r = expect("control_ok.s", "PASS");
    assert!(r.first_failure.is_none());
}

#[test]
fn control_copy_rust_passes() {
    expect("control_copy.rs", "PASS");
}

#[test]
fn overread1_is_oob() {
    let r = expect("overread1.s", "OOB_OR_CRASH");
    assert!(r.first_failure.unwrap().signal.is_some());
}

#[test]
fn overread_is_caught_at_every_length() {
    let r = expect("overread1.s", "OOB_OR_CRASH");
    assert!(input_len(&r) <= 8, "caught only at length {}", input_len(&r));
}

#[test]
fn underread1_is_oob() {
    let r = expect("underread1.s", "OOB_OR_CRASH");
    assert_eq!(r.first_failure.unwrap().placement, "start");
}

#[test]
fn wrong_fe_is_incorrect() {
    let r = expect("wrong_fe.s", "INCORRECT");
    assert!(r.first_failure.unwrap().input_hex.starts_with("fe"));
}

#[test]
fn wrong_fe_reports_shortest() {
    let r = expect("wrong_fe.s", "INCORRECT");
    assert_eq!(input_len(&r), 8);
    assert!(r.mismatches_in_batch > 0);
}

#[test]
fn writes_on_error_is_incorrect() {
    let r = expect("writes_on_error.s", "INCORRECT");
    let f = r.first_failure.unwrap();
    assert_eq!(f.expected.out_written, 0);
    assert_eq!(f.got.unwrap().out_written, 1);
}

#[test]
fn clobber_rbx_is_abi_violation() {
    let r = expect("clobber_rbx.s", "ABI_VIOLATION");
    assert!(r.first_failure.unwrap().abi.is_some());
}

#[test]
fn loop_ff_times_out() {
    let r = expect("loop_ff.s", "TIMEOUT");
    assert!(r.first_failure.unwrap().input_hex.starts_with("ff"));
}

#[test]
fn unsafe_overread_is_oob() {
    expect("unsafe_overread.rs", "OOB_OR_CRASH");
}

#[test]
fn final_seed_depends_on_file() {
    let d = scratch("seed");
    let (a, b) = (d.join("a.s"), d.join("b.s"));
    std::fs::write(&a, "one").unwrap();
    std::fs::write(&b, "two").unwrap();
    let (sa, na) = phase_params(Phase::Final, Some(&a)).unwrap();
    let (sb, _) = phase_params(Phase::Final, Some(&b)).unwrap();
    assert_ne!(sa, sb);
    assert_eq!(na, 20_000_000);
    assert_eq!(sa, seed_from_source(b"one"));
    // first 8 bytes, little-endian, of SHA-256("one")
    assert_eq!(seed_from_source(b""), u64::from_le_bytes([0xe3, 0xb0, 0xc4, 0x42, 0x98, 0xfc, 0x1c, 0x14]));
    assert!(phase_params(Phase::Final, None).is_err());
    assert_eq!(phase_params(Phase::Search, None).unwrap(), (0x5EED0001, 2_000_000));
}

#[test]
fn a_r1_passes_against_itself() {
    setup();
    let search_ops: u64 = manifest_segments("vint", Some("search")).unwrap().iter().map(|s| s.ops).sum();
    let r = check("vint", &a_path(Rung::R1), Phase::Search, None);
    assert_eq!(r.verdict, "PASS", "{r:#?}");
    assert_eq!(r.cases_run, 2 * (33_280 + 2_000_000 + search_ops));
}

#[test]
fn unloadable_candidates_are_tool_errors() {
    use msx_harness::check::try_check;
    setup();
    // exports `fixture`, not `msx_vint`
    let so = support::assemble_fixture("ok_sum.s");
    let e = try_check("vint", &so, Phase::Search, None).unwrap_err();
    assert!(e.contains("could not be loaded") && e.contains("msx_vint"), "{e}");
    // not an ELF file
    let bogus: PathBuf = scratch("bogus").join("bogus.so");
    std::fs::write(&bogus, b"not an elf file").unwrap();
    let e = try_check("vint", &bogus, Phase::Search, None).unwrap_err();
    assert!(e.contains("could not be loaded") && e.contains("bogus.so"), "{e}");
}
