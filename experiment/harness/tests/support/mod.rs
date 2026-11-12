#![allow(dead_code)]
use std::path::PathBuf;
use std::process::Command;

fn work_dir() -> PathBuf {
    match std::env::var_os("MSX_WORK") {
        Some(w) => PathBuf::from(w),
        None => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../work"),
    }
}

/// Assemble `tests/fixtures/<name>` with `gcc -c`, link it with
/// `gcc -shared -nostdlib -nostartfiles` into `$MSX_WORK/tmp/fixtures-<pid>/`, return the `.so`.
/// Every fixture exports the symbol `fixture`.
pub fn assemble_fixture(name: &str) -> PathBuf {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
    let dir = work_dir().join("tmp").join(format!("fixtures-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let stem = src.file_stem().unwrap().to_string_lossy().into_owned();
    let obj = dir.join(format!("{stem}.o"));
    let so = dir.join(format!("{stem}.so"));
    let st = Command::new("gcc").arg("-c").arg(&src).arg("-o").arg(&obj).status().expect("gcc");
    assert!(st.success(), "gcc -c failed for {name}");
    let st = Command::new("gcc")
        .args(["-shared", "-nostdlib", "-nostartfiles"])
        .arg(&obj)
        .arg("-o")
        .arg(&so)
        .status()
        .expect("gcc");
    assert!(st.success(), "gcc -shared failed for {name}");
    so
}

/// Run `body` in a forked child; return the terminating signal if it died of one, else None.
pub fn signal_of_child(body: impl FnOnce()) -> Option<i32> {
    unsafe {
        let pid = libc::fork();
        assert!(pid >= 0);
        if pid == 0 {
            body();
            libc::_exit(0);
        }
        let mut status = 0;
        assert_eq!(libc::waitpid(pid, &mut status, 0), pid);
        if libc::WIFSIGNALED(status) {
            Some(libc::WTERMSIG(status))
        } else {
            None
        }
    }
}

/// `experiment/selftest/vint/<file>`.
pub fn selftest(file: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../selftest/vint").join(file)
}

/// A fresh scratch directory under `$MSX_WORK/tmp/`.
pub fn scratch(tag: &str) -> PathBuf {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let d = work_dir().join("tmp").join(format!("t7-{}-{n}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Build a `selftest/vint` candidate (`.s` with the assembly path, `.rs` with the R2 Rust recipe).
pub fn build_selftest(file: &str) -> Result<PathBuf, msx_harness::build::BuildError> {
    let out = scratch(file);
    if file.ends_with(".s") {
        msx_harness::build::build_asm_candidate("vint", &selftest(file), &out)
    } else {
        msx_harness::build::build_rust_candidate("vint", &selftest(file), msx_harness::build::Rung::R2, &out)
    }
}
