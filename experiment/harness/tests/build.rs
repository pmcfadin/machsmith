use msx_harness::build::*;
use msx_harness::loader::Lib;
use msx_harness::{all_kernels, kernels_with_traces};
use std::path::{Path, PathBuf};
use std::process::Command;

mod support;

fn work() -> PathBuf {
    PathBuf::from(std::env::var_os("MSX_WORK").expect("source env.sh"))
}
fn llvm(tool: &str) -> PathBuf {
    PathBuf::from(std::env::var_os("MSX_LLVM_BIN").expect("source env.sh")).join(tool)
}
fn tmp(name: &str) -> PathBuf {
    let d = work().join("tmp").join(format!("build-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}
fn dyn_symbols(so: &Path) -> String {
    let o = Command::new(llvm("llvm-nm")).args(["-D", "--defined-only"]).arg(so).output().unwrap();
    String::from_utf8_lossy(&o.stdout).into_owned()
}
fn control() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../selftest/vint/control_copy.rs")
}

#[test]
fn r1_and_r2_build_and_export_aligned_symbols() {
    for rung in [Rung::R1, Rung::R2] {
        let so = build_a(rung).unwrap_or_else(|e| panic!("{rung:?}: {} {}", e.stage, e.stderr));
        assert_eq!(so, a_path(rung));
        let syms = dyn_symbols(&so);
        for k in all_kernels() {
            assert!(syms.lines().any(|l| l.ends_with(&format!(" {}", k.symbol()))), "{rung:?} lacks {}", k.symbol());
            assert!(syms.lines().any(|l| l.ends_with(&format!(" {}", k.chain_symbol()))), "{rung:?} lacks chain");
            assert_eq!(entry_alignment(&so, k.symbol()).unwrap(), 0, "{rung:?} {}", k.symbol());
        }
    }
}

#[test]
fn r3_profile_is_real() {
    let so = build_a(Rung::R3).unwrap_or_else(|e| panic!("{} {}", e.stage, e.stderr));
    let merged = work().join("pgo/A/merged.profdata");
    assert!(merged.is_file());
    let o = Command::new(llvm("llvm-profdata")).args(["show", "--all-functions", "--counts"]).arg(&merged).output().unwrap();
    let text = String::from_utf8_lossy(&o.stdout).into_owned();
    let names: Vec<String> = kernels_with_traces().iter().map(|k| k.symbol().to_string()).collect();
    assert!(!names.is_empty());
    let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
    verify_profile(&text, &refs).unwrap();
    for k in all_kernels() {
        assert_eq!(entry_alignment(&so, k.symbol()).unwrap(), 0);
    }
}

#[test]
fn verify_profile_rejects_missing_and_zero() {
    let zero = "  msx_vint:\n    Hash: 0x0baa90d19a7703b5\n    Counters: 7\n    Block counts: [0, 0, 0, 0, 0, 0, 0]\n";
    let e = verify_profile(zero, &["msx_vint"]).unwrap_err();
    assert!(e.contains("msx_vint"), "{e}");
    let e = verify_profile("  other:\n    Hash: 0x1\n    Counters: 1\n    Block counts: [5]\n", &["msx_vint"]).unwrap_err();
    assert!(e.contains("msx_vint"), "{e}");
    let ok = "  msx_vint:\n    Hash: 0x0baa90d19a7703b5\n    Counters: 7\n    Block counts: [0, 13253300, 0, 0, 0, 0, 0]\n";
    verify_profile(ok, &["msx_vint"]).unwrap();
}

#[test]
fn r3copy_matches_r3_recipe() {
    let r3 = build_a(Rung::R3).unwrap_or_else(|e| panic!("{} {}", e.stage, e.stderr));
    let c = build_a(Rung::R3Copy).unwrap_or_else(|e| panic!("{} {}", e.stage, e.stderr));
    assert_ne!(r3, c);
    assert_eq!(disassemble(&r3, "msx_vint").unwrap(), disassemble(&c, "msx_vint").unwrap());
}

#[test]
fn asm_candidate_bad_syntax() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/bad_syntax.s");
    let e = build_asm_candidate("vint", &src, &tmp("bad")).unwrap_err();
    assert_eq!(e.stage, "assemble");
    assert!(!e.stderr.is_empty());
}

#[test]
fn rust_candidate_builds_both_recipes() {
    for (recipe, name) in [(Rung::R2, "cand-r2.so"), (Rung::R3, "cand.so")] {
        let out = tmp(name);
        let so = build_rust_candidate("vint", &control(), recipe, &out).unwrap_or_else(|e| panic!("{} {}", e.stage, e.stderr));
        assert_eq!(so, out.join(name));
        assert_eq!(entry_alignment(&so, "msx_vint").unwrap(), 0);
        assert_eq!(needed_libs(&so).unwrap(), Vec::<String>::new());
    }
}

#[test]
fn rust_candidate_panic_crashes() {
    let out = tmp("panic");
    let src = out.join("panic_cand.rs");
    std::fs::write(
        &src,
        "#[no_mangle]\npub unsafe extern \"C\" fn msx_vint(_p: *const u8, _l: usize, _o: *mut u64) -> usize {\n    panic!()\n}\n",
    )
    .unwrap();
    let so = build_rust_candidate("vint", &src, Rung::R2, &out.join("b")).unwrap_or_else(|e| panic!("{} {}", e.stage, e.stderr));
    let lib = Lib::open(&so).unwrap();
    let f: unsafe extern "C" fn(*const u8, usize, *mut u64) -> usize = unsafe { std::mem::transmute(lib.sym("msx_vint").unwrap()) };
    let sig = support::signal_of_child(|| {
        let mut o = 0u64;
        unsafe { f([0u8].as_ptr(), 1, &mut o) };
    });
    assert_eq!(sig, Some(libc::SIGSEGV));
}

#[test]
fn needed_libs_is_not_vacuous() {
    let so = build_a(Rung::R1).unwrap();
    let libs = needed_libs(&so).unwrap();
    assert!(libs.iter().any(|l| l.starts_with("libc.so")), "{libs:?}");
}
