use msx_harness::lint::{lint_object, lint_source, Arm};

mod support;
use support::{build_selftest, selftest};

fn arm_of(file: &str) -> Arm {
    if file.ends_with(".s") {
        Arm::Asm
    } else {
        Arm::Rust
    }
}

/// Source and object lint together, as `msx attempt` runs them.
fn lint_all(file: &str) -> Vec<String> {
    let arm = arm_of(file);
    let mut errs = lint_source(arm, &selftest(file));
    match build_selftest(file) {
        Ok(so) => errs.extend(lint_object(arm, "vint", &so)),
        Err(e) => errs.push(format!("BUILD({}): {}", e.stage, e.stderr)),
    }
    errs
}

fn assert_rejected(file: &str, needle: &str) {
    let errs = lint_all(file);
    assert!(!errs.is_empty(), "{file} passed the lint");
    assert!(errs.iter().any(|e| e.contains(needle)), "{file}: no message names `{needle}`: {errs:#?}");
}

#[test]
fn syscall_is_rejected() {
    assert_rejected("syscall.s", "syscall");
}

#[test]
fn ctor_is_rejected() {
    assert_rejected("ctor.s", ".init_array");
}

#[test]
fn bigrodata_is_rejected() {
    assert_rejected("bigrodata.s", ".rodata");
}

#[test]
fn init_tag_is_rejected() {
    // `_init` makes the linker emit DT_INIT without any .init section.
    assert_rejected("init_tag.s", "DT_INIT");
    assert_rejected("init_tag.s", "_init");
}

#[test]
fn forbidden_dynamic_tags_are_parsed() {
    let t = "DynamicSection [ (9 entries)\n  Tag Type Name/Value\n  0x000000000000000C INIT 0x1000\n  0x000000000000000D FINI 0x1010\n  0x0000000000000019 INIT_ARRAY 0x2000\n  0x000000000000001E FLAGS 0x0\n  0x0000000000000000 NULL 0x0\n]";
    assert_eq!(msx_harness::lint::forbidden_dynamic_tags(t), vec!["INIT", "FINI", "INIT_ARRAY"]);
    assert!(msx_harness::lint::forbidden_dynamic_tags("  0x0000000000000005 STRTAB 0x270\n").is_empty());
}

#[test]
fn extra_exports_are_rejected() {
    let so = support::build_fixture_candidate("extra_export.s").unwrap_or_else(|e| panic!("{} {}", e.stage, e.stderr));
    let errs = lint_object(Arm::Asm, "vint", &so);
    assert!(errs.iter().any(|e| e.contains("`helper` is exported")), "{errs:#?}");
}

#[test]
fn custom_read_only_sections_count_against_the_cap() {
    // 8192 bytes in `.mytable`, not `.rodata*`: over the 4096 limit for assembly.
    let so = support::build_fixture_candidate("custom_section.s").unwrap_or_else(|e| panic!("{} {}", e.stage, e.stderr));
    let errs = lint_object(Arm::Asm, "vint", &so);
    assert!(errs.iter().any(|e| e.contains(".rodata") && e.contains("limit 4096")), "{errs:#?}");
}

#[test]
fn indirect_is_rejected() {
    assert_rejected("indirect.s", "indirect");
}

#[test]
fn calls_out_is_rejected_by_lint() {
    let errs = lint_all("calls_out.s");
    assert!(!errs.iter().any(|e| e.starts_with("BUILD(")), "calls_out.s failed to build: {errs:#?}");
    assert!(errs.iter().any(|e| e.contains("call")), "{errs:#?}");
    assert!(errs.iter().any(|e| e.contains("undefined symbol")), "{errs:#?}");
}

#[test]
fn uses_asm_is_rejected() {
    let errs = lint_source(Arm::Rust, &selftest("uses_asm.rs"));
    assert!(errs.iter().any(|e| e.contains("asm!")), "{errs:#?}");
}

#[test]
fn controls_and_unsafe_overread_pass() {
    for file in ["control_ok.s", "control_copy.rs", "unsafe_overread.rs"] {
        let arm = arm_of(file);
        let src = lint_source(arm, &selftest(file));
        assert!(src.is_empty(), "{file} source: {src:#?}");
        let so = build_selftest(file).unwrap_or_else(|e| panic!("{file}: {} {}", e.stage, e.stderr));
        let obj = lint_object(arm, "vint", &so);
        assert!(obj.is_empty(), "{file} object: {obj:#?}");
    }
}

#[test]
fn other_sabotage_passes_the_lint() {
    // These misbehave only at run time, so the lint must not be what stops them.
    for file in ["overread1.s", "underread1.s", "wrong_fe.s", "writes_on_error.s", "clobber_rbx.s", "loop_ff.s"] {
        let errs = lint_all(file);
        assert!(errs.is_empty(), "{file}: {errs:#?}");
    }
}

fn src_errors(arm: Arm, text: &str) -> Vec<String> {
    let dir = support::scratch("inline");
    let p = dir.join(if arm == Arm::Asm { "x.s" } else { "x.rs" });
    std::fs::write(&p, text).unwrap();
    lint_source(arm, &p)
}

#[test]
fn asm_source_rules() {
    assert!(!src_errors(Arm::Asm, ".text\n.include \"x.s\"\n").is_empty());
    assert!(!src_errors(Arm::Asm, ".text\n.incbin \"x.bin\"\n").is_empty());
    assert!(!src_errors(Arm::Asm, ".section .data,\"aw\"\n").is_empty());
    assert!(!src_errors(Arm::Asm, ".text ; .include \"x\"\n").is_empty());
    assert!(!src_errors(Arm::Asm, "foo: .data\n").is_empty());
    assert!(src_errors(Arm::Asm, ".text\n# .include \"commented\"\n.section .rodata.cst8,\"a\"\n.section .note.GNU-stack,\"\",@progbits\n").is_empty());
}

#[test]
fn rust_source_rules() {
    for bad in [
        "global_asm!(\"nop\");",
        "#[naked] fn f() {}",
        "static X: &[u8] = include_bytes!(\"a\");",
        "static mut X: u8 = 0;",
        "#[link(name = \"c\")] extern \"C\" {}",
        "#![no_std]",
        "use std::mem;",
        "use alloc::vec::Vec;",
        "extern crate core;",
        "extern \"C\" { fn puts(); }",
        "asm ! (\"nop\");",
        "#[link_section = \".mytable\"] static T: [u8; 4] = [0; 4];",
        "#[unsafe(link_section = \".mytable\")] static T: [u8; 4] = [0; 4];",
    ] {
        assert!(!src_errors(Arm::Rust, bad).is_empty(), "accepted: {bad}");
    }
    let ok = "// asm! in a comment\nuse core::arch::x86_64::_mm_crc32_u8;\npub static mutex_free: u8 = 1;\n#[no_mangle]\npub unsafe extern \"C\" fn msx_vint(p: *const u8, l: usize, o: *mut u64) -> usize { let _ = (p, l, o); 0 }\n";
    assert!(src_errors(Arm::Rust, ok).is_empty(), "{:?}", src_errors(Arm::Rust, ok));
    let two = format!("{ok}pub unsafe extern \"C\" fn msx_other() {{}}\n");
    assert!(!src_errors(Arm::Rust, &two).is_empty());
}
