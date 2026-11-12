//! Rung builds (A.r1, A.r2, A.r3, A.r3copy) with verified PGO, assembly and Rust candidate
//! builds, and entry-alignment checks.

use crate::spec::{all_kernels, kernel, kernels_with_traces};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Rung {
    R1,
    R2,
    R3,
    R3Copy,
}

#[derive(Debug)]
pub struct BuildError {
    pub stage: &'static str,
    pub stderr: String,
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "build stage {}: {}", self.stage, self.stderr)
    }
}

impl std::error::Error for BuildError {}

fn err(stage: &'static str, msg: impl Into<String>) -> BuildError {
    BuildError { stage, stderr: msg.into() }
}

const WRAPPER_LIB: &str = include_str!("../../candidate-wrapper/lib.rs");
const ALIGN_FLAG: &str = "-C llvm-args=-align-all-functions=6";

// ---- locations ----

fn root() -> PathBuf {
    match std::env::var_os("MSX_ROOT") {
        Some(r) => PathBuf::from(r),
        None => Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").components().collect(),
    }
}

pub(crate) fn work() -> PathBuf {
    match std::env::var_os("MSX_WORK") {
        Some(w) => PathBuf::from(w),
        None => root().join("work"),
    }
}

fn llvm_bin() -> PathBuf {
    if let Some(p) = std::env::var_os("MSX_LLVM_BIN") {
        return PathBuf::from(p);
    }
    let out = Command::new("rustc").args(["--print", "sysroot"]).output().expect("rustc --print sysroot");
    PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()).join("lib/rustlib/x86_64-unknown-linux-gnu/bin")
}

pub(crate) fn llvm(tool: &str) -> PathBuf {
    llvm_bin().join(tool)
}

fn rung_dir(rung: Rung) -> &'static str {
    match rung {
        Rung::R1 => "r1",
        Rung::R2 => "r2",
        Rung::R3 => "r3",
        Rung::R3Copy => "r3copy",
    }
}

pub fn a_path(rung: Rung) -> PathBuf {
    work().join("build/A").join(rung_dir(rung)).join("libmsx_kernels.so")
}

fn pgo_dir() -> PathBuf {
    work().join("pgo/A")
}

// ---- process helpers ----

fn run(stage: &'static str, cmd: &mut Command) -> Result<Output, BuildError> {
    let out = cmd.output().map_err(|e| err(stage, format!("cannot run {:?}: {e}", cmd.get_program())))?;
    if !out.status.success() {
        return Err(err(
            stage,
            format!("{:?} failed ({}):\n{}", cmd.get_program(), out.status, String::from_utf8_lossy(&out.stderr)),
        ));
    }
    Ok(out)
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// Serializes all builds: threads in this process and other processes on this host.
struct BuildLock(#[allow(dead_code)] File);

impl BuildLock {
    fn take() -> Result<BuildLock, BuildError> {
        use std::os::fd::AsRawFd;
        let dir = work().join("build");
        std::fs::create_dir_all(&dir).map_err(|e| err("setup", e.to_string()))?;
        let f = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join(".build.lock"))
            .map_err(|e| err("setup", e.to_string()))?;
        if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(err("setup", "flock failed"));
        }
        Ok(BuildLock(f))
    }
}

fn find_msx() -> Result<PathBuf, BuildError> {
    if let Some(p) = std::env::var_os("MSX_BIN") {
        return Ok(PathBuf::from(p));
    }
    let exe = std::env::current_exe().map_err(|e| err("train", e.to_string()))?;
    if exe.file_name().is_some_and(|n| n == "msx") {
        return Ok(exe);
    }
    let mut d = exe.parent();
    for _ in 0..2 {
        if let Some(dir) = d {
            let c = dir.join("msx");
            if c.is_file() {
                return Ok(c);
            }
            d = dir.parent();
        }
    }
    Err(err("train", "cannot find the msx binary (set MSX_BIN)"))
}

// ---- verify_profile, alignment, disassembly ----

/// Parses `llvm-profdata show --all-functions --counts` output. Each named function must be
/// present with at least one nonzero block count.
pub fn verify_profile(show_output: &str, symbols: &[&str]) -> Result<(), String> {
    // (function name, any nonzero block count)
    let mut funcs: Vec<(String, bool)> = Vec::new();
    for line in show_output.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("Block counts:") {
            let nonzero = rest
                .trim()
                .trim_start_matches('[')
                .trim_end_matches(']')
                .split(',')
                .filter_map(|n| n.trim().parse::<u64>().ok())
                .any(|n| n > 0);
            if let Some(last) = funcs.last_mut() {
                last.1 |= nonzero;
            }
        } else if t.ends_with(':') && !t.contains(char::is_whitespace) {
            funcs.push((t.trim_end_matches(':').to_string(), false));
        }
    }
    let mut problems = Vec::new();
    for &sym in symbols {
        let suffix = format!(":{sym}");
        let hits: Vec<&(String, bool)> = funcs.iter().filter(|(n, _)| n == sym || n.ends_with(&suffix)).collect();
        if hits.is_empty() {
            problems.push(format!("{sym}: not present in profile"));
        } else if !hits.iter().any(|h| h.1) {
            problems.push(format!("{sym}: present but every block count is zero"));
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(format!("profile check failed: {}", problems.join("; ")))
    }
}

/// Address of `symbol` in `so`'s dynamic symbol table, modulo 64.
pub fn entry_alignment(so: &Path, symbol: &str) -> Result<u64, String> {
    let out = Command::new(llvm("llvm-nm"))
        .args(["-D", "--defined-only"])
        .arg(so)
        .output()
        .map_err(|e| format!("llvm-nm: {e}"))?;
    if !out.status.success() {
        return Err(format!("llvm-nm failed: {}", text(&out.stderr)));
    }
    for line in text(&out.stdout).lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() == 3 && f[2] == symbol {
            let addr = u64::from_str_radix(f[0], 16).map_err(|e| format!("bad address {}: {e}", f[0]))?;
            return Ok(addr % 64);
        }
    }
    Err(format!("{symbol} is not exported by {}", so.display()))
}

/// DT_NEEDED entries of `so`, from `llvm-readobj --needed-libs`.
pub fn needed_libs(so: &Path) -> Result<Vec<String>, String> {
    let out = Command::new(llvm("llvm-readobj"))
        .arg("--needed-libs")
        .arg(so)
        .output()
        .map_err(|e| format!("llvm-readobj: {e}"))?;
    if !out.status.success() {
        return Err(format!("llvm-readobj failed: {}", text(&out.stderr)));
    }
    let t = text(&out.stdout);
    let start = t.find("NeededLibraries [").ok_or("no NeededLibraries block in llvm-readobj output")?;
    let body = &t[start + "NeededLibraries [".len()..];
    let end = body.find(']').ok_or("unterminated NeededLibraries block")?;
    Ok(body[..end].lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect())
}

fn check_no_needed(so: &Path) -> Result<(), BuildError> {
    let libs = needed_libs(so).map_err(|e| err("needed", e))?;
    if libs.is_empty() {
        Ok(())
    } else {
        Err(err("needed", format!("{} has NEEDED entries: {}", so.display(), libs.join(", "))))
    }
}

/// Disassembly of one symbol with addresses and annotations removed, for comparing builds.
pub fn disassemble(so: &Path, symbol: &str) -> Result<String, String> {
    let out = Command::new(llvm("llvm-objdump"))
        .args(["-d", "--no-show-raw-insn", "--no-addresses"])
        .arg(format!("--disassemble-symbols={symbol}"))
        .arg(so)
        .output()
        .map_err(|e| format!("llvm-objdump: {e}"))?;
    if !out.status.success() {
        return Err(format!("llvm-objdump failed: {}", text(&out.stderr)));
    }
    let mut body = Vec::new();
    for line in text(&out.stdout).lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with("Disassembly of") || t.ends_with(">:") || t.contains("file format") {
            continue;
        }
        // drop `# 0x... <sym>` trailing comments and `0x... <sym+off>` branch annotations
        let t = t.split(" # ").next().unwrap_or(t);
        let t = match t.rfind('<') {
            Some(i) if t.ends_with('>') => t[..i].trim_end(),
            _ => t,
        };
        body.push(t.to_string());
    }
    if body.is_empty() {
        return Err(format!("no disassembly for {symbol} in {}", so.display()));
    }
    Ok(body.join("\n"))
}

// ---- cargo builds of msx-kernels ----

fn rustflags_r1() -> String {
    ALIGN_FLAG.to_string()
}
fn rustflags_r2() -> String {
    format!("{} -C target-cpu=znver5", rustflags_r1())
}

fn cargo_build(target_name: &str, rustflags: &str) -> Result<(PathBuf, String), BuildError> {
    let target = work().join("target").join(target_name);
    let mut cmd = Command::new("cargo");
    cmd.args(["build", "--release", "--lib", "-p", "msx-kernels", "--manifest-path"])
        .arg(root().join("experiment/Cargo.toml"))
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("LLVM_PROFILE_FILE")
        .env("CARGO_TARGET_DIR", &target)
        .env("RUSTFLAGS", rustflags);
    let out = run("cargo", &mut cmd)?;
    Ok((target.join("release/libmsx_kernels.so"), text(&out.stderr)))
}

fn install(built: &Path, rung: Rung) -> Result<PathBuf, BuildError> {
    let dst = a_path(rung);
    std::fs::create_dir_all(dst.parent().unwrap()).map_err(|e| err("install", e.to_string()))?;
    std::fs::copy(built, &dst).map_err(|e| err("install", format!("{}: {e}", built.display())))?;
    Ok(dst)
}

fn check_a_alignment(so: &Path) -> Result<(), BuildError> {
    for k in all_kernels() {
        for sym in [k.symbol(), k.chain_symbol()] {
            entry_alignment(so, sym).map_err(|e| err("export", e))?;
        }
        let a = entry_alignment(so, k.symbol()).map_err(|e| err("align", e))?;
        if a != 0 {
            return Err(err("align", format!("{} entry is at offset {a} mod 64 in {}", k.symbol(), so.display())));
        }
    }
    Ok(())
}

fn sha256_file(p: &Path) -> Result<String, BuildError> {
    use sha2::{Digest, Sha256};
    let b = std::fs::read(p).map_err(|e| err("hash", format!("{}: {e}", p.display())))?;
    Ok(Sha256::digest(&b).iter().map(|x| format!("{x:02x}")).collect())
}

/// Run a trained instrumented library to produce raw profiles in `raw`, merge to `merged`, and
/// return the `show --all-functions --counts` text.
fn train_and_merge(so: &Path, kernels: &str, raw: &Path, merged: &Path) -> Result<String, BuildError> {
    let _ = std::fs::remove_dir_all(raw);
    std::fs::create_dir_all(raw).map_err(|e| err("train", e.to_string()))?;
    let _ = std::fs::remove_file(merged);
    let mut cmd = Command::new(find_msx()?);
    cmd.args(["__train", "--so"])
        .arg(so)
        .args(["--kernels", kernels])
        .env("LLVM_PROFILE_FILE", raw.join("%m.profraw"));
    run("train", &mut cmd)?;
    let any = std::fs::read_dir(raw)
        .map_err(|e| err("train", e.to_string()))?
        .filter_map(|e| e.ok())
        .any(|e| e.path().extension().is_some_and(|x| x == "profraw"));
    if !any {
        return Err(err("train", format!("training wrote no .profraw into {}", raw.display())));
    }
    run("merge", Command::new(llvm("llvm-profdata")).arg("merge").arg("-o").arg(merged).arg(raw))?;
    let show = run(
        "merge",
        Command::new(llvm("llvm-profdata")).args(["show", "--all-functions", "--counts"]).arg(merged),
    )?;
    Ok(text(&show.stdout))
}

/// Fail on profile-use diagnostics that mean PGO did nothing for a trained function.
fn check_use_stderr(stderr: &str, allow_missing_untraced: bool) -> Result<(), BuildError> {
    let untraced: Vec<&str> = if allow_missing_untraced {
        let traced: Vec<&str> = kernels_with_traces().iter().map(|k| k.name()).collect();
        all_kernels().iter().map(|k| k.name()).filter(|n| !traced.contains(n)).collect()
    } else {
        Vec::new()
    };
    for line in stderr.lines() {
        if line.contains("hash mismatch") {
            return Err(err("profile-use", format!("profile hash mismatch: {line}")));
        }
        if line.contains("no profile data available for function msx_") {
            let tolerated = untraced.iter().any(|n| line.contains(&format!("function msx_{n}")));
            if !tolerated {
                return Err(err("profile-use", format!("PGO did nothing for a kernel: {line}")));
            }
        }
    }
    Ok(())
}

fn build_r3_family(rung: Rung) -> Result<PathBuf, BuildError> {
    let pgo = pgo_dir();
    let merged = pgo.join("merged.profdata");
    if rung == Rung::R3 || !merged.is_file() {
        let flags = format!("{} -C profile-generate={}", rustflags_r2(), pgo.join("raw").display());
        let (gen_so, _) = cargo_build("A-r3-gen", &flags)?;
        let show = train_and_merge(&gen_so, "all", &pgo.join("raw"), &merged)?;
        let names: Vec<String> = kernels_with_traces().iter().map(|k| k.symbol().to_string()).collect();
        let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        verify_profile(&show, &refs).map_err(|e| err("profile-verify", e))?;
    }
    // Cargo's fingerprint does not see the profile's contents: force a rebuild when they change.
    let target_name = if rung == Rung::R3 { "A-r3" } else { "A-r3copy" };
    let target = work().join("target").join(target_name);
    let stamp = target.join(".profdata.sha256");
    let sha = sha256_file(&merged)?;
    if std::fs::read_to_string(&stamp).ok().as_deref() != Some(sha.as_str()) {
        let _ = std::fs::remove_dir_all(target.join("release"));
    }
    let flags = format!(
        "{} -C profile-use={} -C llvm-args=-pgo-warn-missing-function",
        rustflags_r2(),
        merged.display()
    );
    let (so, stderr) = cargo_build(target_name, &flags)?;
    check_use_stderr(&stderr, true)?;
    std::fs::create_dir_all(&target).map_err(|e| err("setup", e.to_string()))?;
    std::fs::write(&stamp, &sha).map_err(|e| err("setup", e.to_string()))?;
    install(&so, rung)
}

pub fn build_a(rung: Rung) -> Result<PathBuf, BuildError> {
    let _lock = BuildLock::take()?;
    let path = match rung {
        Rung::R1 => install(&cargo_build("A-r1", &rustflags_r1())?.0, rung)?,
        Rung::R2 => install(&cargo_build("A-r2", &rustflags_r2())?.0, rung)?,
        Rung::R3 | Rung::R3Copy => build_r3_family(rung)?,
    };
    check_a_alignment(&path)?;
    Ok(path)
}

// ---- candidates ----

fn check_candidate_export(so: &Path, kernel_name: &str) -> Result<(), BuildError> {
    let sym = format!("msx_{kernel_name}");
    entry_alignment(so, &sym).map_err(|e| err("export", e))?;
    check_no_needed(so)
}

pub fn build_asm_candidate(kernel_name: &str, src: &Path, out_dir: &Path) -> Result<PathBuf, BuildError> {
    kernel(kernel_name).ok_or_else(|| err("kernel", format!("unknown kernel {kernel_name}")))?;
    std::fs::create_dir_all(out_dir).map_err(|e| err("setup", e.to_string()))?;
    let obj = out_dir.join("cand.o");
    let so = out_dir.join("cand.so");
    run("assemble", Command::new("gcc").args(["-c", "-x", "assembler", "-o"]).arg(&obj).arg(src))?;
    run(
        "link",
        Command::new("gcc")
            .args(["-shared", "-nostdlib", "-nostartfiles", "-Wl,-z,noexecstack", "-Wl,--build-id=none", "-o"])
            .arg(&so)
            .arg(&obj),
    )?;
    check_candidate_export(&so, kernel_name)?;
    Ok(so)
}

fn rustc_common(cmd: &mut Command) {
    cmd.args([
        "--edition",
        "2021",
        "--crate-name",
        "msx_cand",
        "-C",
        "opt-level=3",
        "-C",
        "codegen-units=1",
        "-C",
        "lto=fat",
        "-C",
        "panic=abort",
        "-C",
        "target-cpu=znver5",
        "-C",
        "llvm-args=-align-all-functions=6",
        "--check-cfg=cfg(msx_train)",
    ]);
}

pub fn build_rust_candidate(kernel_name: &str, src: &Path, recipe: Rung, out_dir: &Path) -> Result<PathBuf, BuildError> {
    kernel(kernel_name).ok_or_else(|| err("kernel", format!("unknown kernel {kernel_name}")))?;
    if recipe != Rung::R2 && recipe != Rung::R3 {
        return Err(err("recipe", "Rust candidates are built with recipe R2 or R3"));
    }
    let _lock = BuildLock::take()?;
    let out_dir = if out_dir.is_absolute() { out_dir.to_path_buf() } else { std::env::current_dir().unwrap().join(out_dir) };
    let srcdir = out_dir.join("src");
    std::fs::create_dir_all(&srcdir).map_err(|e| err("setup", e.to_string()))?;
    std::fs::write(srcdir.join("lib.rs"), WRAPPER_LIB).map_err(|e| err("setup", e.to_string()))?;
    std::fs::copy(src, srcdir.join("candidate.rs")).map_err(|e| err("setup", format!("{}: {e}", src.display())))?;

    let mut profile_use: Option<PathBuf> = None;
    if recipe == Rung::R3 {
        let pgo = out_dir.join("pgo");
        let raw = pgo.join("raw");
        let merged = pgo.join("merged.profdata");
        let _ = std::fs::remove_dir_all(&pgo);
        std::fs::create_dir_all(&raw).map_err(|e| err("setup", e.to_string()))?;
        let train_so = out_dir.join("train.so");
        let mut cmd = Command::new("rustc");
        cmd.current_dir(&srcdir).args(["--crate-type", "cdylib", "--cfg", "msx_train"]);
        rustc_common(&mut cmd);
        cmd.arg("-C").arg(format!("profile-generate={}", raw.display())).arg("lib.rs").arg("-o").arg(&train_so);
        run("rustc-train", &mut cmd)?;
        let show = train_and_merge(&train_so, kernel_name, &raw, &merged)?;
        verify_profile(&show, &[&format!("msx_{kernel_name}")]).map_err(|e| err("profile-verify", e))?;
        profile_use = Some(merged);
    }

    let archive = out_dir.join("cand.a");
    let mut cmd = Command::new("rustc");
    cmd.current_dir(&srcdir).args(["--crate-type", "staticlib"]);
    rustc_common(&mut cmd);
    if let Some(m) = &profile_use {
        cmd.arg("-C").arg(format!("profile-use={}", m.display()));
        cmd.args(["-C", "llvm-args=-pgo-warn-missing-function"]);
    }
    cmd.arg("lib.rs").arg("-o").arg(&archive);
    let out = run("rustc", &mut cmd)?;
    if profile_use.is_some() {
        // The panic handler exists only in this no_std build: its missing-profile warning is expected.
        let stderr: String = text(&out.stderr)
            .lines()
            .filter(|l| !l.contains("rust_begin_unwind"))
            .collect::<Vec<_>>()
            .join("\n");
        check_use_stderr(&stderr, false)?;
    }

    let so = out_dir.join(if recipe == Rung::R3 { "cand.so" } else { "cand-r2.so" });
    run(
        "link",
        Command::new("gcc")
            .args(["-shared", "-nostdlib", "-nostartfiles", "-Wl,--gc-sections"])
            .arg(format!("-Wl,-u,msx_{kernel_name}"))
            .args(["-Wl,-z,noexecstack", "-Wl,--build-id=none", "-o"])
            .arg(&so)
            .arg(&archive),
    )?;
    check_candidate_export(&so, kernel_name)?;
    Ok(so)
}

// ---- helpers for the CLI ----

pub fn sha256_hex(p: &Path) -> Result<String, BuildError> {
    sha256_file(p)
}
