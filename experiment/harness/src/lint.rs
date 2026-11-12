//! Static lint of candidate source and candidate shared object. Runs before any candidate code is loaded.
//!
//! Every function returns a list of human-readable problems; an empty list means pass. Messages
//! start with the rule that fired and cite the offending line of source or disassembly.

use crate::build::{llvm, needed_libs};
use crate::check::{FILE_MAX, RODATA_MAX_ASM, RODATA_MAX_RUST, TEXT_MAX};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Arm {
    Asm,
    Rust,
}

/// Libc functions the Rust compiler emits calls to; the C library already loaded in the process
/// resolves them, exactly as for the `A` builds.
const RUST_ALLOWED_UNDEF: [&str; 5] = ["memcpy", "memmove", "memset", "memcmp", "bcmp"];

// ======================================================================================
// Source lint
// ======================================================================================

pub fn lint_source(arm: Arm, src: &Path) -> Vec<String> {
    let text = match std::fs::read_to_string(src) {
        Ok(t) => t,
        Err(e) => return vec![format!("source: cannot read {}: {e}", src.display())],
    };
    match arm {
        Arm::Asm => lint_asm_source(&text),
        Arm::Rust => lint_rust_source(&text),
    }
}

fn lint_asm_source(text: &str) -> Vec<String> {
    let mut errs = Vec::new();
    let clean = strip_asm_comments(text);
    for (i, line) in clean.lines().enumerate() {
        let shown = text.lines().nth(i).unwrap_or("").trim();
        for stmt in line.split(';') {
            let mut s = stmt.trim();
            // peel leading labels (`foo:` and `1:`)
            while let Some(colon) = s.find(':') {
                let head = &s[..colon];
                if !head.is_empty() && head.chars().all(|c| c.is_alphanumeric() || "_.$".contains(c)) {
                    s = s[colon + 1..].trim_start();
                } else {
                    break;
                }
            }
            if s.is_empty() {
                continue;
            }
            let (dir, rest) = match s.find(|c: char| c.is_whitespace()) {
                Some(k) => (&s[..k], s[k..].trim()),
                None => (s, ""),
            };
            let dir = dir.to_ascii_lowercase();
            match dir.as_str() {
                ".include" | ".incbin" => errs.push(format!("directive {dir} is not allowed: line {}: {shown}", i + 1)),
                ".section" | ".pushsection" => {
                    let name = rest.split(',').next().unwrap_or("").trim().trim_matches('"');
                    let ok = name == ".text"
                        || name == ".rodata"
                        || name.starts_with(".rodata.")
                        || name == ".note.GNU-stack";
                    if !ok || dir == ".pushsection" {
                        errs.push(format!("section `{name}` is not allowed (only .text, .rodata*, .note.GNU-stack): line {}: {shown}", i + 1));
                    }
                }
                ".data" | ".bss" | ".tdata" | ".tbss" | ".comm" | ".lcomm" | ".subsection" | ".previous" | ".popsection"
                | ".init_array" | ".fini_array" | ".ctors" | ".dtors" => {
                    errs.push(format!("directive {dir} is not allowed (writable or initializer section): line {}: {shown}", i + 1));
                }
                _ => {}
            }
        }
    }
    errs
}

/// Removes `#` line comments and `/* */` block comments, keeping line structure. Quoted strings
/// are respected so `#` inside a `.section` flags string does not truncate a line.
fn strip_asm_comments(text: &str) -> String {
    let b: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let (mut i, mut in_str, mut in_block) = (0, false, false);
    while i < b.len() {
        let c = b[i];
        if in_block {
            if c == '*' && b.get(i + 1) == Some(&'/') {
                in_block = false;
                i += 2;
                continue;
            }
            if c == '\n' {
                out.push('\n');
            }
            i += 1;
            continue;
        }
        if in_str {
            out.push(c);
            if c == '\\' {
                if let Some(n) = b.get(i + 1) {
                    out.push(*n);
                    i += 1;
                }
            } else if c == '"' || c == '\n' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        match c {
            '"' => {
                in_str = true;
                out.push(c);
            }
            '/' if b.get(i + 1) == Some(&'*') => {
                in_block = true;
                i += 1;
            }
            '#' => {
                while i < b.len() && b[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            _ => out.push(c),
        }
        i += 1;
    }
    out
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    Str(String),
    P(char),
}

/// Tokens of Rust source with their line numbers; comments are dropped.
fn rust_tokens(text: &str) -> Vec<(Tok, usize)> {
    let b: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let (mut i, mut line) = (0usize, 1usize);
    while i < b.len() {
        let c = b[i];
        if c == '\n' {
            line += 1;
            i += 1;
        } else if c.is_whitespace() {
            i += 1;
        } else if c == '/' && b.get(i + 1) == Some(&'/') {
            while i < b.len() && b[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && b.get(i + 1) == Some(&'*') {
            let mut depth = 1;
            i += 2;
            while i < b.len() && depth > 0 {
                if b[i] == '/' && b.get(i + 1) == Some(&'*') {
                    depth += 1;
                    i += 2;
                } else if b[i] == '*' && b.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    i += 2;
                } else {
                    if b[i] == '\n' {
                        line += 1;
                    }
                    i += 1;
                }
            }
        } else if c == '"' {
            let start_line = line;
            let mut s = String::new();
            i += 1;
            while i < b.len() && b[i] != '"' {
                if b[i] == '\\' {
                    i += 1;
                }
                if i < b.len() {
                    if b[i] == '\n' {
                        line += 1;
                    }
                    s.push(b[i]);
                }
                i += 1;
            }
            i += 1;
            out.push((Tok::Str(s), start_line));
        } else if c.is_alphanumeric() || c == '_' {
            let st = i;
            while i < b.len() && (b[i].is_alphanumeric() || b[i] == '_') {
                i += 1;
            }
            out.push((Tok::Ident(b[st..i].iter().collect()), line));
        } else {
            out.push((Tok::P(c), line));
            i += 1;
        }
    }
    out
}

fn lint_rust_source(text: &str) -> Vec<String> {
    let toks = rust_tokens(text);
    let lines: Vec<&str> = text.lines().collect();
    let shown = |l: usize| lines.get(l - 1).map(|s| s.trim()).unwrap_or("");
    let mut errs = Vec::new();
    let id = |k: usize| -> Option<&str> {
        match toks.get(k) {
            Some((Tok::Ident(s), _)) => Some(s.as_str()),
            _ => None,
        }
    };
    let p = |k: usize, c: char| matches!(toks.get(k), Some((Tok::P(x), _)) if *x == c);
    let mut externs_ok = 0usize;
    for (k, (t, l)) in toks.iter().enumerate() {
        let l = *l;
        let mut bad = |rule: &str| errs.push(format!("forbidden `{rule}`: line {l}: {}", shown(l)));
        match t {
            Tok::Ident(w) => {
                if (w == "asm" || w.ends_with("_asm")) && p(k + 1, '!') {
                    bad("asm!");
                } else if w == "naked" || w == "naked_asm" {
                    bad("naked");
                } else if w == "link_section" {
                    bad("link_section");
                } else if matches!(w.as_str(), "include_bytes" | "include_str" | "include") && p(k + 1, '!') {
                    bad(&format!("{w}!"));
                } else if w == "static" && id(k + 1) == Some("mut") {
                    bad("static mut");
                } else if (w == "std" || w == "alloc") && p(k + 1, ':') && p(k + 2, ':') {
                    bad(&format!("{w}::"));
                } else if w == "extern" && id(k + 1) == Some("crate") {
                    bad("extern crate");
                } else if w == "extern" {
                    let is_def = matches!(toks.get(k + 1), Some((Tok::Str(s), _)) if s == "C")
                        && id(k + 2) == Some("fn")
                        && id(k + 3).is_some_and(|n| n.starts_with("msx_") && n.len() > 4);
                    if is_def {
                        externs_ok += 1;
                        if externs_ok > 1 {
                            bad("extern (more than one `extern \"C\" fn msx_<kernel>` definition)");
                        }
                    } else {
                        bad("extern (only `extern \"C\" fn msx_<kernel>` is allowed)");
                    }
                }
            }
            Tok::P('#') => {
                if p(k + 1, '!') && p(k + 2, '[') {
                    bad("#![");
                } else if p(k + 1, '[') {
                    if id(k + 2) == Some("link") {
                        bad("#[link");
                    } else if id(k + 2) == Some("unsafe") && p(k + 3, '(') && id(k + 4) == Some("link") {
                        bad("#[link");
                    }
                }
            }
            _ => {}
        }
    }
    errs
}

// ======================================================================================
// Object lint
// ======================================================================================

#[derive(Debug, Clone)]
struct Section {
    name: String,
    size: u64,
    addr: u64,
    write: bool,
    exec: bool,
    alloc: bool,
}

fn run_tool(tool: &str, args: &[&str], path: &Path) -> Result<String, String> {
    let out = Command::new(llvm(tool))
        .args(args)
        .arg(path)
        .output()
        .map_err(|e| format!("lint: cannot run {tool}: {e}"))?;
    if !out.status.success() {
        return Err(format!("lint: {tool} failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn parse_sections(t: &str) -> Vec<Section> {
    let mut out = Vec::new();
    let mut cur: Option<Section> = None;
    for line in t.lines() {
        let l = line.trim();
        if l == "Section {" {
            cur = Some(Section { name: String::new(), size: 0, addr: 0, write: false, exec: false, alloc: false });
        } else if let Some(s) = cur.as_mut() {
            if let Some(n) = l.strip_prefix("Name: ") {
                s.name = n.rsplit_once(" (").map(|x| x.0).unwrap_or(n).to_string();
            } else if let Some(v) = l.strip_prefix("Size: ") {
                s.size = v.trim().parse().unwrap_or(0);
            } else if let Some(v) = l.strip_prefix("Address: ") {
                s.addr = u64::from_str_radix(v.trim().trim_start_matches("0x"), 16).unwrap_or(0);
            } else if l.starts_with("SHF_WRITE") {
                s.write = true;
            } else if l.starts_with("SHF_EXECINSTR") {
                s.exec = true;
            } else if l.starts_with("SHF_ALLOC") {
                s.alloc = true;
            } else if l == "}" {
                out.extend(cur.take());
            }
        }
    }
    out
}

fn has_prefix_name(name: &str, base: &str) -> bool {
    name == base || (name.starts_with(base) && name[base.len()..].starts_with('.'))
}

/// (slot address -> (relocation type, symbol)) from `--dyn-relocations`.
fn parse_dyn_relocs(t: &str) -> Vec<(u64, String, String)> {
    let mut out = Vec::new();
    for line in t.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() >= 3 && f[0].starts_with("0x") && f[1].starts_with("R_X86_64_") {
            if let Ok(a) = u64::from_str_radix(&f[0][2..], 16) {
                out.push((a, f[1].to_string(), f[2].to_string()));
            }
        }
    }
    out
}

const PREFIXES: [&str; 12] =
    ["rep", "repz", "repe", "repnz", "repne", "lock", "notrack", "bnd", "data16", "cs", "ds", "xacquire"];

const FORBIDDEN_MNEMONICS: [&str; 10] =
    ["syscall", "sysenter", "int", "rdtsc", "rdtscp", "rdpmc", "rdrand", "rdseed", "cpuid", "xgetbv"];

struct Insn<'a> {
    addr: u64,
    mnemonic: String,
    operands: &'a str,
    comment_addr: Option<u64>,
    line: &'a str,
}

fn parse_insn(line: &str) -> Option<Insn<'_>> {
    let t = line.trim();
    let (a, rest) = t.split_once(':')?;
    let addr = u64::from_str_radix(a.trim(), 16).ok()?;
    let rest = rest.trim();
    let (body, comment) = match rest.split_once('#') {
        Some((b, c)) => (b.trim(), Some(c.trim())),
        None => (rest, None),
    };
    let mut parts = body.split_whitespace();
    let mut m = parts.next()?.to_ascii_lowercase();
    let mut operands = body[m.len()..].trim();
    while PREFIXES.contains(&m.as_str()) {
        let after = operands;
        let mut p2 = after.split_whitespace();
        match p2.next() {
            Some(nm) => {
                m = nm.to_ascii_lowercase();
                operands = after[nm.len()..].trim();
            }
            None => break,
        }
    }
    let comment_addr = comment.and_then(|c| {
        let tok = c.split_whitespace().next()?;
        u64::from_str_radix(tok.strip_prefix("0x")?, 16).ok()
    });
    Some(Insn { addr, mnemonic: m, operands, comment_addr, line: t })
}

/// Dynamic-table tags that make the loader run code before the kernel is called.
const FORBIDDEN_DYNAMIC_TAGS: [&str; 6] = ["INIT", "FINI", "INIT_ARRAY", "FINI_ARRAY", "PREINIT_ARRAY", "PREINIT_ARRAYSZ"];

/// Tag names of `llvm-readobj --dynamic-table` that are forbidden (see `FORBIDDEN_DYNAMIC_TAGS`).
pub fn forbidden_dynamic_tags(t: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in t.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() >= 2 && f[0].starts_with("0x") && FORBIDDEN_DYNAMIC_TAGS.contains(&f[1]) && !out.contains(&f[1].to_string()) {
            out.push(f[1].to_string());
        }
    }
    out
}

/// Names that the dynamic linking machinery needs and that carry no kernel data.
const LINKING_SECTIONS: [&str; 10] =
    [".dynsym", ".dynstr", ".gnu.hash", ".hash", ".dynamic", ".rela.dyn", ".rela.plt", ".got", ".got.plt", ".eh_frame"];

/// True for the sections that do not count against the read-only data cap.
fn is_linking_section(name: &str) -> bool {
    LINKING_SECTIONS.contains(&name) || name == ".eh_frame_hdr" || name.starts_with(".note.") || name == ".note" || name.starts_with(".gnu.version")
}

/// Bytes of read-only data: every allocated section that is neither executable nor writable, except
/// the ones dynamic linking needs. A custom section name must not hide a table from the cap.
fn read_only_bytes(secs: &[Section]) -> u64 {
    secs.iter().filter(|s| s.alloc && !s.exec && !s.write && !is_linking_section(&s.name)).map(|s| s.size).sum()
}

fn is_branch(m: &str) -> bool {
    m.starts_with('j') || m.starts_with("call") || m.starts_with("loop")
}

pub fn lint_object(arm: Arm, kernel: &str, so: &Path) -> Vec<String> {
    let mut errs = Vec::new();
    macro_rules! tool {
        ($e:expr) => {
            match $e {
                Ok(v) => v,
                Err(e) => {
                    errs.push(e);
                    return errs;
                }
            }
        };
    }
    let symbol = format!("msx_{kernel}");

    // file size
    match std::fs::metadata(so) {
        Ok(m) if m.len() > FILE_MAX => errs.push(format!("file size {} bytes exceeds {FILE_MAX}", m.len())),
        Ok(_) => {}
        Err(e) => {
            errs.push(format!("lint: cannot stat {}: {e}", so.display()));
            return errs;
        }
    }

    // exported function
    let nm = tool!(run_tool("llvm-nm", &["-D", "--defined-only", "-S"], so));
    let mut found = None;
    for line in nm.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() == 4 && f[3] == symbol {
            let addr = u64::from_str_radix(f[0], 16).unwrap_or(0);
            let size = u64::from_str_radix(f[1], 16).unwrap_or(0);
            found = Some((addr, size, f[2].to_string()));
        }
    }
    // Nothing but the kernel may be exported: another global (for example `_init`) is a way to run
    // or hide code that the lint would not see as the kernel.
    for line in nm.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if let Some(name) = f.last() {
            if f.len() >= 3 && *name != symbol {
                errs.push(format!("export: `{name}` is exported; only {symbol} may be: {}", line.trim()));
            }
        }
    }
    match found {
        Some((addr, size, ty)) if matches!(ty.as_str(), "T" | "t" | "W" | "w") && size > 0 => {
            if addr % 64 != 0 {
                errs.push(format!("entry alignment: {symbol} is at {addr:#x}, which is {} mod 64 (must be 0)", addr % 64));
            }
            if size > TEXT_MAX {
                errs.push(format!("text size: {symbol} is {size} bytes, limit {TEXT_MAX}"));
            }
        }
        Some((_, size, ty)) => errs.push(format!("export: {symbol} must be a function with nonzero size (type {ty}, size {size})")),
        None => errs.push(format!("export: {symbol} is not exported as a function")),
    }

    // NEEDED
    match needed_libs(so) {
        Ok(l) if !l.is_empty() => errs.push(format!("NEEDED libraries are not allowed: {}", l.join(", "))),
        Ok(_) => {}
        Err(e) => errs.push(format!("lint: {e}")),
    }

    // load-time code (DT_INIT and friends have no section of their own)
    let dynt = tool!(run_tool("llvm-readobj", &["--dynamic-table"], so));
    for tag in forbidden_dynamic_tags(&dynt) {
        errs.push(format!("dynamic tag DT_{tag} is present (load-time code is not allowed)"));
    }

    // undefined symbols
    let und = tool!(run_tool("llvm-nm", &["-D", "--undefined-only"], so));
    for line in und.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        let Some(name) = f.last() else { continue };
        if f.len() < 2 {
            continue;
        }
        let base = name.split('@').next().unwrap_or(name);
        let ok = arm == Arm::Rust && RUST_ALLOWED_UNDEF.contains(&base);
        if !ok {
            errs.push(format!("undefined symbol `{base}`: {}", line.trim()));
        }
    }

    // dynamic relocations
    let relocs_txt = tool!(run_tool("llvm-readobj", &["--dyn-relocations"], so));
    let relocs = parse_dyn_relocs(&relocs_txt);
    let mut slots: BTreeMap<u64, String> = BTreeMap::new();
    for (addr, ty, sym) in &relocs {
        let ok = arm == Arm::Rust
            && (ty == "R_X86_64_GLOB_DAT" || ty == "R_X86_64_JUMP_SLOT")
            && RUST_ALLOWED_UNDEF.contains(&sym.as_str());
        if ok {
            slots.insert(*addr, sym.clone());
        } else {
            errs.push(format!("dynamic relocation not allowed: {addr:#x} {ty} {sym}"));
        }
    }

    // sections
    let secs = parse_sections(&tool!(run_tool("llvm-readobj", &["--sections"], so)));
    for s in &secs {
        if [".init", ".fini", ".init_array", ".fini_array", ".preinit_array", ".ctors", ".dtors"]
            .iter()
            .any(|b| has_prefix_name(&s.name, b))
        {
            errs.push(format!("initializer section {} is present", s.name));
        }
        let datalike = [".data", ".bss", ".tdata", ".tbss"].iter().any(|b| has_prefix_name(&s.name, b));
        if datalike && s.size > 0 {
            errs.push(format!("writable data section {} has size {}", s.name, s.size));
        } else if s.write && s.size > 0 && !matches!(s.name.as_str(), ".dynamic" | ".got" | ".got.plt") {
            errs.push(format!("writable section {} has size {}", s.name, s.size));
        }
    }
    let rodata = read_only_bytes(&secs);
    let rodata_max = if arm == Arm::Asm { RODATA_MAX_ASM } else { RODATA_MAX_RUST };
    if rodata > rodata_max {
        errs.push(format!(".rodata (all read-only allocated sections) total is {rodata} bytes, limit {rodata_max}"));
    }

    // instructions
    let dis = tool!(run_tool("llvm-objdump", &["-d", "--no-show-raw-insn"], so));
    let exec: Vec<&Section> = secs.iter().filter(|s| s.exec && s.size > 0).collect();
    let in_exec = |a: u64| exec.iter().any(|s| a >= s.addr && a < s.addr + s.size);
    let insns: Vec<Insn> = dis.lines().filter_map(parse_insn).collect();
    let boundaries: BTreeSet<u64> = insns.iter().map(|i| i.addr).collect();
    for line in dis.lines() {
        if line.contains("(bad)") {
            errs.push(format!("undecodable bytes in an executable section: {}", line.trim()));
        }
    }
    for i in &insns {
        let m = i.mnemonic.as_str();
        let forbidden = FORBIDDEN_MNEMONICS.contains(&m) || m.starts_with("rdrand") || m.starts_with("rdseed");
        if forbidden {
            errs.push(format!("forbidden instruction `{m}`: {}", i.line));
            continue;
        }
        if !is_branch(m) {
            continue;
        }
        if arm == Arm::Asm && m.starts_with("call") {
            errs.push(format!("call in an assembly candidate (leaf code only): {}", i.line));
        }
        if i.operands.starts_with('*') {
            let via_slot = i.comment_addr.and_then(|a| slots.get(&a));
            if arm == Arm::Rust && via_slot.is_some() {
                continue;
            }
            errs.push(format!("indirect {m}: {}", i.line));
        } else if let Some(h) = i.operands.split_whitespace().next().and_then(|t| t.strip_prefix("0x")) {
            match u64::from_str_radix(h, 16) {
                Ok(target) if !in_exec(target) => {
                    errs.push(format!("{m} target {target:#x} is outside the file's executable sections: {}", i.line))
                }
                Ok(target) if !boundaries.contains(&target) => {
                    errs.push(format!("{m} target {target:#x} is not an instruction boundary: {}", i.line))
                }
                _ => {}
            }
        } else {
            errs.push(format!("unrecognised {m} operand: {}", i.line));
        }
    }
    errs
}
