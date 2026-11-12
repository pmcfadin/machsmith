mod support;
use msx_harness::loader::Lib;
use msx_harness::tramp::{call4, upper_state_check_available, AbiViolation};

fn load(name: &str) -> (Lib, msx_harness::RawFn) {
    let lib = Lib::open(&support::assemble_fixture(name)).unwrap();
    let f = lib.sym("fixture").expect("symbol");
    (lib, f)
}

#[test]
fn ok_sum_passes() {
    let (_l, f) = load("ok_sum.s");
    assert_eq!(unsafe { call4(f, 1, 2, 3, 4) }, Ok(10));
    assert_eq!(unsafe { call4(f, u64::MAX, 1, 0, 0) }, Ok(0));
}

#[test]
fn clobbered_rbx_is_caught() {
    let (_l, f) = load("clobber_rbx.s");
    assert_eq!(unsafe { call4(f, 0, 0, 0, 0) }, Err(AbiViolation::CalleeSaved));
}

#[test]
fn direction_flag_is_caught_and_cleared() {
    let (_l, f) = load("set_df.s");
    assert_eq!(unsafe { call4(f, 0, 0, 0, 0) }, Err(AbiViolation::DirectionFlag));
    let src: Vec<u8> = (0..64).collect();
    let mut dst = vec![0u8; 64];
    dst.copy_from_slice(&src);
    assert_eq!(src, dst);
}

#[test]
fn dirty_ymm_is_caught() {
    if !upper_state_check_available() {
        eprintln!("NOTICE: CPUID.(0DH,1):EAX bit 2 absent; skipping dirty_ymm");
        return;
    }
    let (_l, f) = load("dirty_ymm.s");
    assert_eq!(unsafe { call4(f, 0, 0, 0, 0) }, Err(AbiViolation::UpperVectorState));
    // The trampoline cleans up after itself: a following clean call passes.
    let (_l2, g) = load("ok_sum.s");
    assert_eq!(unsafe { call4(g, 1, 1, 1, 1) }, Ok(4));
}

#[test]
fn mxcsr_change_is_caught_and_restored() {
    let (_l, f) = load("set_mxcsr.s");
    assert_eq!(unsafe { call4(f, 0, 0, 0, 0) }, Err(AbiViolation::Mxcsr));
    let (_l2, g) = load("ok_sum.s");
    assert_eq!(unsafe { call4(g, 1, 1, 1, 1) }, Ok(4)); // MXCSR was restored by the trampoline
}

#[test]
fn moved_stack_pointer_is_caught() {
    let (_l, f) = load("bad_rsp.s");
    assert_eq!(unsafe { call4(f, 0, 0, 0, 0) }, Err(AbiViolation::StackPointer));
}
