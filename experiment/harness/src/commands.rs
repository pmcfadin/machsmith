//! The `attempt`, `null` and `final` commands. They hold an exclusive `flock` on `work/msx.lock`
//! from start to finish, so only one build-check-measure sequence runs on the host at a time.
//!
//! Failure policy: a *tool* error (bad usage, dirty tree, missing rung build, a candidate that
//! cannot even be loaded, a measurement that fails for a reason other than the noise gate) is an
//! `Err` that `main` turns into exit code 2 and writes no ledger record. A *verdict* (lint, build,
//! check, budget, noise gate) is recorded and printed with exit code 0.

use crate::build::{a_path, build_asm_candidate, build_rust_candidate, root, runtime_verdict_of_stage, rustc_version, sha256_hex, work, Rung};
use crate::check::{try_check, CheckReport, Phase};
use crate::ledger::{self, budget, COUNTED_ATTEMPTS, MAX_SUBMISSIONS};
use crate::lint::{lint_object, lint_source, Arm};
use crate::measure::{
    is_candidate_name, run_protocol, size_override_active, Contestant, ContestantFailure, Protocol, RunResult, Split, FINAL_BLOCKS, FINAL_SESSIONS, NULL_BAND_MAX, SEARCH_BLOCKS,
    SEARCH_SESSIONS, THRESHOLD_FLOOR,
};
use crate::spec::{kernel, kernels_with_traces, KernelSpec};
use crate::stats::{null_band, threshold, verdict, Interval};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------------------------
// Pure helpers (unit-tested)
// ---------------------------------------------------------------------------------------------

fn unquote(p: &str) -> String {
    let p = p.trim();
    match p.strip_prefix('"').and_then(|q| q.strip_suffix('"')) {
        Some(inner) => inner.replace("\\\"", "\"").replace("\\\\", "\\"),
        None => p.to_string(),
    }
}

/// Checks `git status --porcelain` output (ideally with `--untracked-files=all`): everything
/// changed or untracked must lie inside `experiment/candidates/<kernel>/<arm>/` or be the ledger
/// (or its lock file). Returns the offending paths. Both ends of a rename are examined.
pub fn tree_is_clean_for(porcelain: &str, kernel: &str, arm: &str) -> Result<(), Vec<String>> {
    let prefix = format!("experiment/candidates/{kernel}/{arm}/");
    let mut bad: Vec<String> = Vec::new();
    for line in porcelain.lines() {
        if line.len() < 4 {
            continue;
        }
        let rest = &line[3..];
        let paths: Vec<String> = match rest.split_once(" -> ") {
            Some((a, b)) => vec![unquote(a), unquote(b)],
            None => vec![unquote(rest)],
        };
        for p in paths {
            let traversal = p.split('/').any(|c| c == ".." || c == ".");
            let ok = !traversal
                && (p == "experiment/results/ledger.jsonl"
                    || p == "experiment/results/ledger.lock"
                    || (p.starts_with(&prefix) && p.len() > prefix.len()));
            if !ok && !bad.contains(&p) {
                bad.push(p);
            }
        }
    }
    if bad.is_empty() {
        Ok(())
    } else {
        Err(bad)
    }
}

/// `FINAL: attempt-07` on the first non-empty line -> 7. Anything else -> None.
pub fn parse_final_md(text: &str) -> Option<u32> {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    let n = line.strip_prefix("FINAL: attempt-")?;
    if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    n.parse().ok()
}

// ---------------------------------------------------------------------------------------------
// Shared plumbing
// ---------------------------------------------------------------------------------------------

fn arg_after(args: &[String], flag: &str) -> Option<String> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned()
}

fn traced_kernel(args: &[String]) -> Result<(String, &'static dyn KernelSpec), String> {
    let name = arg_after(args, "--kernel").ok_or("--kernel required")?;
    let k = kernel(&name).ok_or(format!("unknown kernel {name}"))?;
    if !kernels_with_traces().iter().any(|t| t.name() == name) {
        return Err(format!("{name}: no recorded traces, nothing to measure"));
    }
    Ok((name, k))
}

fn refuse_size_override() -> Result<(), String> {
    if size_override_active() {
        return Err("MSX_SESSIONS/MSX_BLOCKS are set: size overrides never produce ledger records".into());
    }
    Ok(())
}

/// Environment variables that change how a candidate is checked, run or judged; they exist for
/// tests and never for a ledgered run. (`MSX_LEDGER` is allowed and recorded instead.)
pub const REFUSED_ENV: [&str; 4] = ["MSX_CHECK_TIMEOUT_SECS", "MSX_EXE", "MSX_BIN", "MSX_SELFTEST_DIR"];

/// The members of `REFUSED_ENV` that `is_set` reports as set.
pub fn refused_env_set(is_set: impl Fn(&str) -> bool) -> Vec<&'static str> {
    REFUSED_ENV.iter().copied().filter(|n| is_set(n)).collect()
}

/// Size overrides and `REFUSED_ENV` make `attempt`, `null` and `final` refuse (exit 2, no record).
fn refuse_overrides() -> Result<(), String> {
    refuse_size_override()?;
    let set = refused_env_set(|n| std::env::var_os(n).is_some());
    if !set.is_empty() {
        return Err(format!("{} set: these overrides never produce ledger records", set.join(", ")));
    }
    Ok(())
}

/// The `MSX_LEDGER` override, as recorded in every record the commands write.
fn ledger_override() -> Value {
    match std::env::var_os("MSX_LEDGER") {
        Some(p) => Value::String(p.to_string_lossy().into_owned()),
        None => Value::Null,
    }
}

fn host_lock() -> Result<std::fs::File, String> {
    eprintln!("msx: waiting for the host lock (work/msx.lock)");
    ledger::lock_exclusive(&work().join("msx.lock"))
}

fn arm_name(arm: Arm) -> &'static str {
    match arm {
        Arm::Asm => "asm",
        Arm::Rust => "rust",
    }
}

fn git_porcelain() -> Result<String, String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root())
        .args(["status", "--porcelain", "--untracked-files=all"])
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if !out.status.success() {
        return Err(format!("git status failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn contestant(name: &str, so: PathBuf, k: &dyn KernelSpec) -> Contestant {
    Contestant { name: name.into(), so, symbol: k.symbol().into(), is_chain: false }
}

fn require_a(rungs: &[Rung]) -> Result<(), String> {
    for r in rungs {
        let p = a_path(*r);
        if !p.is_file() {
            return Err(format!("{} is missing; run `msx rungs build`", p.display()));
        }
    }
    Ok(())
}

fn is_noise(e: &str) -> bool {
    e.starts_with("NOISE_GATE_FAILED")
}

fn rel_to_root(p: &Path) -> String {
    let r = root().canonicalize().unwrap_or_else(|_| root());
    p.strip_prefix(&r).unwrap_or(p).to_string_lossy().into_owned()
}

fn iv(res: &RunResult, cand: &str, base: &str) -> Option<Interval> {
    res.pair(cand, base).and_then(|p| p.interval)
}

fn comparison(res: &RunResult, cand: &str, base: &str, t: f64) -> Value {
    let i = iv(res, cand, base);
    json!({ "cand": cand, "base": base, "interval": i, "verdict": i.map(|i| verdict(i, t)).unwrap_or("INCONCLUSIVE") })
}

fn sizes(protocol: Protocol, res: Option<&RunResult>) -> Value {
    match (res, protocol) {
        (Some(r), _) => json!({ "sessions": r.sessions_used, "blocks": r.blocks_used }),
        (None, Protocol::Search) => json!({ "sessions": SEARCH_SESSIONS, "blocks": SEARCH_BLOCKS }),
        (None, Protocol::Final) => json!({ "sessions": FINAL_SESSIONS, "blocks": FINAL_BLOCKS }),
    }
}

// ---------------------------------------------------------------------------------------------
// attempt
// ---------------------------------------------------------------------------------------------

fn nonempty_env(name: &str) -> Result<String, String> {
    match std::env::var(name) {
        Ok(v) if !v.trim().is_empty() => Ok(v),
        _ => Err(format!("{name} is not set; attempts are recorded per agent and model")),
    }
}

pub fn attempt(args: &[String]) -> Result<Value, String> {
    let kname = arg_after(args, "--kernel").ok_or("--kernel required")?;
    let arm = match arg_after(args, "--arm").ok_or("--arm required")?.as_str() {
        "asm" => Arm::Asm,
        "rust" => Arm::Rust,
        o => return Err(format!("bad --arm {o} (asm|rust)")),
    };
    let file = arg_after(args, "--file").ok_or("--file required")?;
    let arm_s = arm_name(arm);

    // 1. identity and overrides
    let agent_id = nonempty_env("MSX_AGENT_ID")?;
    let model = nonempty_env("MSX_MODEL")?;
    refuse_overrides()?;
    let (kname, k) = traced_kernel(&[String::from("--kernel"), kname])?;

    let _host = host_lock()?;

    // 2. clean tree
    if let Err(paths) = tree_is_clean_for(&git_porcelain()?, &kname, arm_s) {
        return Err(format!(
            "the working tree has changes outside experiment/candidates/{kname}/{arm_s}/ and the ledger:\n  {}",
            paths.join("\n  ")
        ));
    }

    // 3. file check
    let ledger_before = ledger::read_all();
    let b = budget(&ledger_before, &kname, arm_s);
    let submission = b.submissions + 1;
    let cand_dir = root().join("experiment/candidates").join(&kname).join(arm_s);
    let cand_dir = cand_dir.canonicalize().map_err(|e| format!("{}: {e}", cand_dir.display()))?;
    let path = Path::new(&file).canonicalize().map_err(|e| format!("{file}: {e}"))?;
    if path.parent() != Some(cand_dir.as_path()) {
        return Err(format!("{file} must be a file directly inside {}", cand_dir.display()));
    }
    let ext = if arm == Arm::Asm { "s" } else { "rs" };
    let want = format!("attempt-{submission:02}.{ext}");
    if path.file_name().and_then(|n| n.to_str()) != Some(want.as_str()) {
        return Err(format!("this is submission {submission}: the file must be named {want}"));
    }
    let sha = sha256_hex(&path).map_err(|e| e.to_string())?;
    let prior: Vec<&Value> = ledger_before
        .iter()
        .filter(|r| r["kind"] == "attempt" && r["kernel"] == kname.as_str() && r["arm"] == arm_s && r["candidate_sha256"] == sha.as_str())
        .collect();
    if !prior.is_empty() && !prior.iter().all(|r| r["noise_failure"] == true) {
        return Err(format!("duplicate submission: these exact bytes were already submitted (sha256 {sha})"));
    }
    let cand_rel = rel_to_root(&path);

    // 4. budget
    let base = |counted: Value, verdict_s: &str| {
        json!({
            "kind": "attempt", "kernel": kname, "arm": arm_s, "agent_id": agent_id, "model": model,
            "submission": submission, "counted_attempt": counted, "candidate_path": cand_rel,
            "candidate_sha256": sha, "verdict": verdict_s, "lint": [], "build": null, "check": null,
            "measure": null, "noise_failure": false, "protocol_sizes": null,
            "rustc_version": rustc_version(), "ledger_override": ledger_override(),
        })
    };
    if b.submissions >= MAX_SUBMISSIONS || b.counted >= COUNTED_ATTEMPTS {
        let mut rec = base(Value::Null, "BUDGET_EXHAUSTED");
        rec["kind"] = "refused".into();
        rec["reason"] = format!("submissions {}/{MAX_SUBMISSIONS}, counted {}/{COUNTED_ATTEMPTS}", b.submissions, b.counted).into();
        ledger::append(rec)?;
        return Ok(json!({ "verdict": "BUDGET_EXHAUSTED", "submissions_left": MAX_SUBMISSIONS.saturating_sub(b.submissions), "counted_attempts_left": COUNTED_ATTEMPTS.saturating_sub(b.counted) }));
    }

    let left = |counted: u32| (COUNTED_ATTEMPTS - counted, MAX_SUBMISSIONS - submission);
    // A free failure: lint or build.
    let free_failure = |verdict_s: &str, lint: Vec<String>, build: Value| -> Result<Value, String> {
        let mut rec = base(Value::Null, verdict_s);
        rec["lint"] = json!(lint);
        rec["build"] = build.clone();
        ledger::append(rec)?;
        let (cl, sl) = left(b.counted);
        Ok(json!({
            "submission": submission, "counted_attempt": null, "counted_attempts_left": cl, "submissions_left": sl,
            "verdict": verdict_s, "lint_errors": lint, "build_stderr": build["stderr"], "first_failure": null, "timing": null,
        }))
    };

    // A counted attempt that failed at run time before or after the check (the training run of a
    // Rust candidate, the measurement). `fields` are merged into the record.
    let counted_no = b.counted + 1;
    let counted_failure = |verdict_s: &str, detail: &str, fields: Vec<(&str, Value)>| -> Result<Value, String> {
        let mut rec = base(counted_no.into(), verdict_s);
        for (k, v) in fields {
            rec[k] = v;
        }
        ledger::append(rec)?;
        let (cl, sl) = left(counted_no);
        Ok(json!({
            "submission": submission, "counted_attempt": counted_no, "counted_attempts_left": cl, "submissions_left": sl,
            "verdict": verdict_s, "lint_errors": [], "build_stderr": null, "first_failure": null, "timing": null, "detail": detail,
        }))
    };

    // 5. lint, build, lint the object
    let errs = lint_source(arm, &path);
    if !errs.is_empty() {
        return free_failure("LINT_FAILED", errs, Value::Null);
    }
    let out_dir = work().join("build/cand").join(&kname).join(arm_s).join(format!("attempt-{submission:02}"));
    let built = match arm {
        Arm::Asm => build_asm_candidate(&kname, &path, &out_dir),
        Arm::Rust => build_rust_candidate(&kname, &path, Rung::R3, &out_dir),
    };
    let so = match built {
        Ok(so) => so,
        Err(e) if e.stage == "setup" => return Err(e.to_string()),
        // The candidate's own code ran (PGO training) and crashed or hung: a counted attempt.
        Err(e) if runtime_verdict_of_stage(e.stage).is_some() => {
            let v = runtime_verdict_of_stage(e.stage).unwrap_or("OOB_OR_CRASH");
            return counted_failure(v, &e.stderr, vec![("build", json!({ "stage": "train", "detail": e.stderr }))]);
        }
        Err(e) => return free_failure("BUILD_FAILED", vec![], json!({ "stage": e.stage, "stderr": e.stderr })),
    };
    let errs = lint_object(arm, &kname, &so);
    if !errs.is_empty() {
        return free_failure("LINT_FAILED", errs, Value::Null);
    }

    // 6. correctness (this attempt counts from here on). A tool error here writes no record.
    require_a(&[Rung::R1, Rung::R2, Rung::R3])?;
    let report: CheckReport = try_check(&kname, &so, Phase::Search, None)?;
    let (cl, sl) = left(counted_no);
    let mut rec = base(counted_no.into(), &report.verdict);
    rec["check"] = serde_json::to_value(&report).map_err(|e| e.to_string())?;
    let mut out = json!({
        "submission": submission, "counted_attempt": counted_no, "counted_attempts_left": cl, "submissions_left": sl,
        "verdict": report.verdict, "lint_errors": [], "build_stderr": null,
        "first_failure": report.first_failure, "timing": null,
    });
    if report.verdict != "PASS" {
        rec["protocol_sizes"] = Value::Null;
        ledger::append(rec)?;
        return Ok(out);
    }

    // 7. timing
    let contestants = vec![
        contestant("A.r1", a_path(Rung::R1), k),
        contestant("A.r2", a_path(Rung::R2), k),
        contestant("A.r3", a_path(Rung::R3), k),
        contestant("cand", so.clone(), k),
    ];
    match run_protocol(&kname, &contestants, Protocol::Search, Split::Search) {
        Ok(res) => {
            let med = |f: fn(&crate::measure::Medians) -> f64| -> Value {
                let mut m = serde_json::Map::new();
                for c in &contestants {
                    if let Some(x) = res.medians.get(&c.name) {
                        m.insert(c.name.clone(), f(x).into());
                    }
                }
                Value::Object(m)
            };
            let mut ratio = serde_json::Map::new();
            for rung in ["A.r1", "A.r2", "A.r3"] {
                ratio.insert(rung.into(), serde_json::to_value(iv(&res, "cand", rung)).map_err(|e| e.to_string())?);
            }
            out["timing"] = json!({
                "provisional": true,
                "cycles_per_op": med(|m| m.cycles_per_op), "instructions_per_op": med(|m| m.instructions_per_op),
                "branch_misses_per_op": med(|m| m.branch_misses_per_op), "ns_per_op": med(|m| m.ns_per_op),
                "ratio_vs": ratio,
            });
            rec["measure"] = serde_json::to_value(&res).map_err(|e| e.to_string())?;
            rec["protocol_sizes"] = sizes(Protocol::Search, Some(&res));
            ledger::append(rec)?;
            Ok(out)
        }
        Err(e) if is_noise(&e) => {
            // The measurement could not be taken; the attempt does not count and the same source may
            // be resubmitted under the next submission number.
            rec["counted_attempt"] = Value::Null;
            rec["verdict"] = "NOISE_GATE_FAILED".into();
            rec["noise_failure"] = Value::Bool(true);
            rec["noise_detail"] = e.clone().into();
            rec["protocol_sizes"] = sizes(Protocol::Search, None);
            ledger::append(rec)?;
            let (cl, sl) = left(b.counted);
            out["counted_attempt"] = Value::Null;
            out["counted_attempts_left"] = cl.into();
            out["submissions_left"] = sl.into();
            out["verdict"] = "NOISE_GATE_FAILED".into();
            out["detail"] = e.into();
            Ok(out)
        }
        // The candidate diverged from the reference, died or hung while being timed: the check
        // did not cover that input, but the attempt reached the check and counts.
        Err(e) => match ContestantFailure::parse(&e).filter(|f| is_candidate_name(&f.contestant)) {
            Some(f) => counted_failure(
                &f.verdict,
                &f.detail,
                vec![
                    ("check", serde_json::to_value(&report).map_err(|e| e.to_string())?),
                    ("measure_failure", json!({ "contestant": f.contestant, "verdict": f.verdict, "detail": f.detail })),
                ],
            ),
            None => Err(e),
        },
    }
}

// ---------------------------------------------------------------------------------------------
// null
// ---------------------------------------------------------------------------------------------

fn reasm_path(kname: &str) -> PathBuf {
    work().join("build/A/reasm").join(format!("{kname}.so"))
}

/// Null intervals keyed `"A.r3copy/A.r3"`, ... from a run that contains A.r3copy and A.r3 (and A.reasm).
fn null_intervals(res: &RunResult, with_reasm: bool) -> Result<Vec<(String, Interval)>, String> {
    let mut v = Vec::new();
    let mut pairs = vec![("A.r3copy", "A.r3")];
    if with_reasm {
        pairs.push(("A.reasm", "A.r3"));
    }
    for (c, b) in pairs {
        let i = iv(res, c, b).ok_or(format!("no interval for {c}/{b}"))?;
        v.push((format!("{c}/{b}"), i));
    }
    Ok(v)
}

fn nulls_json(nulls: &[(String, Interval)]) -> Value {
    let mut m = serde_json::Map::new();
    for (n, i) in nulls {
        m.insert(n.clone(), serde_json::to_value(i).unwrap_or(Value::Null));
    }
    Value::Object(m)
}

pub fn null(args: &[String]) -> Result<Value, String> {
    let (kname, k) = traced_kernel(args)?;
    refuse_overrides()?;
    let _host = host_lock()?;
    require_a(&[Rung::R3, Rung::R3Copy])?;
    let reasm = reasm_path(&kname);
    let with_reasm = reasm.is_file();
    let mut cs = vec![contestant("A.r3", a_path(Rung::R3), k), contestant("A.r3copy", a_path(Rung::R3Copy), k)];
    if with_reasm {
        cs.push(contestant("A.reasm", reasm, k));
    }
    match run_protocol(&kname, &cs, Protocol::Final, Split::Search) {
        Ok(res) => {
            let nulls = null_intervals(&res, with_reasm)?;
            let ivs: Vec<Interval> = nulls.iter().map(|(_, i)| *i).collect();
            let band = null_band(&ivs);
            let pass = band <= NULL_BAND_MAX;
            let mut rec = json!({
                "kind": "null", "kernel": kname, "arm": null, "nulls": nulls_json(&nulls), "band": band,
                "threshold": threshold(band), "pass": pass, "noise_failure": false,
                "measure": res, "protocol_sizes": sizes(Protocol::Final, Some(&res)),
                "ledger_override": ledger_override(),
            });
            let id = ledger::append(rec.clone())?;
            rec.as_object_mut().map(|o| o.remove("measure"));
            rec["id"] = id.clone().into();
            write_json(&root().join("experiment/results/null").join(format!("{kname}-{id}.json")), &rec)?;
            Ok(rec)
        }
        Err(e) if is_noise(&e) => {
            let rec = json!({
                "kind": "null", "kernel": kname, "arm": null, "nulls": null, "band": null, "threshold": null,
                "pass": false, "noise_failure": true, "noise_detail": e, "verdict": "NOISE_GATE_FAILED",
                "measure": null, "protocol_sizes": sizes(Protocol::Final, None), "ledger_override": ledger_override(),
            });
            let id = ledger::append(rec.clone())?;
            let mut rec = rec;
            rec["id"] = id.clone().into();
            write_json(&root().join("experiment/results/null").join(format!("{kname}-{id}.json")), &rec)?;
            Ok(rec)
        }
        Err(e) => Err(e),
    }
}

fn write_json(path: &Path, v: &Value) -> Result<(), String> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    std::fs::write(path, serde_json::to_string_pretty(v).map_err(|e| e.to_string())? + "\n").map_err(|e| format!("{}: {e}", path.display()))
}

// ---------------------------------------------------------------------------------------------
// final
// ---------------------------------------------------------------------------------------------

struct Finalist {
    so: PathBuf,
    r2: Option<PathBuf>,
}

/// Why `final` may not run again: a final that issued verdicts (status `OK`) is never repeated; a
/// rerun after a measured final needs a reason; and a rerun may not change any arm's pick (the
/// attempt chosen in `FINAL.md`) from the latest final record. `picks` is `(arm, chosen attempt)`
/// as `FINAL.md` names it now.
pub fn check_rerun(ledger: &[Value], kernel: &str, picks: &[(&str, Option<u32>)], rerun_reason: Option<&str>) -> Result<(), String> {
    let finals: Vec<&Value> = ledger.iter().filter(|r| r["kind"] == "final" && r["kernel"] == kernel).collect();
    let Some(latest) = finals.last() else { return Ok(()) };
    if finals.iter().any(|r| r["status"] == "OK") {
        return Err(format!("the ledger already holds a final for {kernel} that issued verdicts; a second final is never allowed"));
    }
    let measured_before = finals.iter().any(|r| !r["measure"].is_null());
    if measured_before && rerun_reason.map_or(true, |r| r.trim().is_empty()) {
        return Err(format!(
            "the ledger already holds a measured final for {kernel}; a rerun is legitimate only after a tool failure or NOISE_GATE_FAILED, and needs --rerun-reason TEXT"
        ));
    }
    for (arm, pick) in picks {
        let before = latest["arms"][*arm]["attempt"].as_u64().map(|n| n as u32);
        if before != *pick {
            let show = |p: Option<u32>| p.map_or("no pick".to_string(), |n| format!("attempt-{n:02}"));
            return Err(format!(
                "a rerun may not change the pick: {arm} was {} in {}, FINAL.md now says {}",
                show(before),
                latest["id"].as_str().unwrap_or("the previous final"),
                show(*pick)
            ));
        }
    }
    Ok(())
}

/// The final record's noise-gate failure: status, the flag and the detail agree.
pub fn mark_noise_gate_failed(rec: &mut Value, detail: String) {
    rec["status"] = "NOISE_GATE_FAILED".into();
    rec["noise_failure"] = Value::Bool(true);
    rec["noise_detail"] = detail.into();
}

/// The R2-recipe build of the finalist's source (`B.rust.r2`) is a contestant too: it must pass the
/// same object lint and final check as the entry, or it is left out of the measurement. `Err` is
/// the reason to record.
pub fn vet_r2(kernel: &str, r2_so: &Path, src: &Path) -> Result<(), String> {
    let lint = lint_object(Arm::Rust, kernel, r2_so);
    if !lint.is_empty() {
        return Err(format!("lint: {}", lint.join("; ")));
    }
    match try_check(kernel, r2_so, Phase::Final, Some(src)) {
        Ok(rep) if rep.verdict == "PASS" => Ok(()),
        Ok(rep) => Err(format!("final check: {} ({})", rep.verdict, serde_json::to_string(&rep.first_failure).unwrap_or_default())),
        Err(e) => Err(format!("check could not run: {e}")),
    }
}

pub fn final_exam(args: &[String]) -> Result<Value, String> {
    let (kname, k) = traced_kernel(args)?;
    let rerun_reason = arg_after(args, "--rerun-reason");
    refuse_overrides()?;
    let _host = host_lock()?;

    // 1. one final that issues verdicts per kernel; a rerun needs a reason and may not change picks
    let ledger_before = ledger::read_all();
    let picks: Vec<(&str, Option<u32>)> = ["asm", "rust"]
        .iter()
        .map(|a| {
            let dir = root().join("experiment/candidates").join(&kname).join(a);
            (*a, std::fs::read_to_string(dir.join("FINAL.md")).ok().and_then(|t| parse_final_md(&t)))
        })
        .collect();
    check_rerun(&ledger_before, &kname, &picks, rerun_reason.as_deref())?;

    // 2-4. finalists
    let mut arms = serde_json::Map::new();
    let mut finalists: Vec<(Arm, Finalist)> = Vec::new();
    for arm in [Arm::Asm, Arm::Rust] {
        let arm_s = arm_name(arm);
        let dir = root().join("experiment/candidates").join(&kname).join(arm_s);
        let no_entry = |why: String, n: Option<u32>| json!({ "status": "NO_ENTRY", "reason": why, "attempt": n });
        let Ok(text) = std::fs::read_to_string(dir.join("FINAL.md")) else {
            arms.insert(arm_s.into(), no_entry("FINAL.md is missing".into(), None));
            continue;
        };
        let Some(n) = parse_final_md(&text) else {
            arms.insert(arm_s.into(), no_entry("FINAL.md is not `FINAL: attempt-NN`".into(), None));
            continue;
        };
        let Some(rec) = ledger_before
            .iter()
            .rev()
            .find(|r| r["kind"] == "attempt" && r["kernel"] == kname.as_str() && r["arm"] == arm_s && r["submission"] == n)
            .filter(|r| r["verdict"] == "PASS")
        else {
            arms.insert(arm_s.into(), no_entry(format!("attempt-{n:02} has no PASS record in the ledger"), Some(n)));
            continue;
        };
        let cand_rel = rec["candidate_path"].as_str().ok_or("ledger record without candidate_path")?.to_string();
        let src = root().join(&cand_rel);
        let mut entry = json!({ "attempt": n, "candidate_path": cand_rel, "candidate_sha256": rec["candidate_sha256"] });
        let disq = |mut e: Value, why: String| {
            e["status"] = "DISQUALIFIED".into();
            e["reason"] = why.into();
            e
        };
        let sha_now = sha256_hex(&src).map_err(|e| format!("{}: {e}", src.display()))?;
        if rec["candidate_sha256"] != sha_now.as_str() {
            arms.insert(arm_s.into(), disq(entry, "the source changed after its PASS attempt (sha256 differs from the ledger)".into()));
            continue;
        }
        let out_dir = work().join("build/cand").join(&kname).join(arm_s).join(format!("attempt-{n:02}"));
        let built = match arm {
            Arm::Asm => build_asm_candidate(&kname, &src, &out_dir),
            Arm::Rust => build_rust_candidate(&kname, &src, Rung::R3, &out_dir),
        };
        let so = match built {
            Ok(so) => so,
            // Its PGO training run crashed or hung this time: the candidate is disqualified.
            Err(e) if runtime_verdict_of_stage(e.stage).is_some() => {
                let v = runtime_verdict_of_stage(e.stage).unwrap_or("OOB_OR_CRASH");
                arms.insert(arm_s.into(), disq(entry, format!("the PGO training run of the rebuild: {v}: {}", e.stderr)));
                continue;
            }
            Err(e) => return Err(format!("rebuilding the finalist {}: {e}", cand_rel)),
        };
        let report = try_check(&kname, &so, Phase::Final, Some(&src))?;
        entry["check"] = serde_json::to_value(&report).map_err(|e| e.to_string())?;
        if report.verdict != "PASS" {
            arms.insert(arm_s.into(), disq(entry, format!("final check: {}", report.verdict)));
            continue;
        }
        // The R2-recipe build of the same source is a contestant too (B.rust.r2): it gets the same
        // lint and final check, and is left out of the measurement when it fails either.
        let mut r2: Option<PathBuf> = None;
        if arm == Arm::Rust {
            let vetted = build_rust_candidate(&kname, &src, Rung::R2, &out_dir).map_err(|e| format!("build: {e}")).and_then(|so2| {
                vet_r2(&kname, &so2, &src)?;
                Ok(so2)
            });
            match vetted {
                Ok(so2) => r2 = Some(so2),
                Err(why) => entry["r2_omitted"] = why.into(),
            }
        }
        entry["status"] = "ENTERED".into();
        arms.insert(arm_s.into(), entry);
        finalists.push((arm, Finalist { so, r2 }));
    }

    let mut rec = json!({
        "kind": "final", "kernel": kname, "arm": null, "arms": null, "rerun_reason": rerun_reason,
        "measure": null, "nulls": null, "band": null, "threshold": null, "verdicts": null, "boundary_tax": null,
        "noise_failure": false, "protocol_sizes": null, "status": "NO_FINALISTS",
        "rustc_version": rustc_version(), "ledger_override": ledger_override(),
    });

    if !finalists.is_empty() {
        require_a(&[Rung::R1, Rung::R2, Rung::R3, Rung::R3Copy])?;
    }
    // 5. contestants; a finalist that diverges, crashes or hangs while being timed is disqualified
    // (with the reason) and the protocol is run again without it.
    while !finalists.is_empty() {
        let reasm = reasm_path(&kname);
        let with_reasm = reasm.is_file();
        let mut cs = vec![
            contestant("A.r1", a_path(Rung::R1), k),
            contestant("A.r2", a_path(Rung::R2), k),
            contestant("A.r3", a_path(Rung::R3), k),
            contestant("A.r3copy", a_path(Rung::R3Copy), k),
        ];
        if with_reasm {
            cs.push(contestant("A.reasm", reasm, k));
        }
        cs.push(Contestant { name: "A.inline".into(), so: a_path(Rung::R3), symbol: k.chain_symbol().into(), is_chain: true });
        for (arm, f) in &finalists {
            match arm {
                Arm::Asm => cs.push(contestant("B.asm", f.so.clone(), k)),
                Arm::Rust => {
                    cs.push(contestant("B.rust", f.so.clone(), k));
                    if let Some(r2) = &f.r2 {
                        cs.push(contestant("B.rust.r2", r2.clone(), k));
                    }
                }
            }
        }
        match run_protocol(&kname, &cs, Protocol::Final, Split::Holdout) {
            Ok(res) => {
                // 6. nulls and threshold
                let nulls = null_intervals(&res, with_reasm)?;
                let ivs: Vec<Interval> = nulls.iter().map(|(_, i)| *i).collect();
                let band = null_band(&ivs);
                let t = threshold(band);
                rec["nulls"] = nulls_json(&nulls);
                rec["band"] = band.into();
                rec["threshold"] = t.into();
                rec["measure"] = serde_json::to_value(&res).map_err(|e| e.to_string())?;
                rec["protocol_sizes"] = sizes(Protocol::Final, Some(&res));
                if band > NULL_BAND_MAX {
                    mark_noise_gate_failed(&mut rec, format!("null band {band:.4} > {NULL_BAND_MAX}"));
                } else {
                    // 7. verdicts
                    rec["status"] = "OK".into();
                    let has = |n: &str| cs.iter().any(|c| c.name == n);
                    let mut vs = Vec::new();
                    for b in ["B.asm", "B.rust", "B.rust.r2"] {
                        if has(b) {
                            for a in ["A.r1", "A.r2", "A.r3"] {
                                vs.push(comparison(&res, b, a, t));
                            }
                        }
                    }
                    if has("B.asm") && has("B.rust") {
                        vs.push(comparison(&res, "B.asm", "B.rust", t));
                    }
                    vs.push(comparison(&res, "A.r2", "A.r1", t));
                    vs.push(comparison(&res, "A.r3", "A.r2", t));
                    rec["verdicts"] = Value::Array(vs);
                    rec["boundary_tax"] = json!({ "cand": "A.r3", "base": "A.inline", "interval": iv(&res, "A.r3", "A.inline") });
                }
                break;
            }
            Err(e) if is_noise(&e) => {
                mark_noise_gate_failed(&mut rec, e);
                rec["protocol_sizes"] = sizes(Protocol::Final, None);
                break;
            }
            Err(e) => {
                let Some(f) = ContestantFailure::parse(&e).filter(|f| is_candidate_name(&f.contestant)) else { return Err(e) };
                let why = format!("during measurement: {}: {}", f.verdict, f.detail);
                let failure = json!({ "contestant": f.contestant, "verdict": f.verdict, "detail": f.detail });
                match f.contestant.as_str() {
                    "B.rust.r2" => {
                        // only the R2 build of the Rust entry is out; B.rust stays
                        if let Some(a) = arms.get_mut("rust") {
                            a["r2_omitted"] = why.into();
                            a["r2_measure_failure"] = failure;
                        }
                        for (arm, fin) in finalists.iter_mut() {
                            if *arm == Arm::Rust {
                                fin.r2 = None;
                            }
                        }
                    }
                    name => {
                        let (arm, arm_s) = if name == "B.asm" { (Arm::Asm, "asm") } else { (Arm::Rust, "rust") };
                        if let Some(a) = arms.get_mut(arm_s) {
                            a["status"] = "DISQUALIFIED".into();
                            a["reason"] = why.into();
                            a["measure_failure"] = failure;
                        }
                        finalists.retain(|(x, _)| *x != arm);
                    }
                }
            }
        }
    }

    rec["arms"] = Value::Object(arms);
    let id = ledger::append(rec.clone())?;
    rec["id"] = id.into();
    write_json(&root().join("experiment/results/final").join(format!("{kname}.json")), &rec)?;
    if let Some(o) = rec.as_object_mut() {
        o.remove("measure");
    }
    Ok(rec)
}

// ---------------------------------------------------------------------------------------------
// selftest
// ---------------------------------------------------------------------------------------------

/// What a self-test file must produce. `check` is a check verdict or `LINT_FAILED`; `measured` is
/// set only for controls (`PASS/LOST` -> `Some("LOST")`, `PASS/*` -> `Some("*")`, any verdict).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expect {
    pub check: String,
    pub measured: Option<String>,
}

const CHECK_VERDICTS: [&str; 7] = ["LINT_FAILED", "BUILD_FAILED", "INCORRECT", "OOB_OR_CRASH", "ABI_VIOLATION", "TIMEOUT", "PASS"];
const MEASURE_VERDICTS: [&str; 4] = ["WON", "LOST", "TIED", "INCONCLUSIVE"];

/// Parses an `EXPECT.json`: an object mapping file name to `"<check>"` or `"PASS/<measured>"`.
pub fn parse_expect(text: &str) -> Result<std::collections::BTreeMap<String, Expect>, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("EXPECT.json: {e}"))?;
    let obj = v.as_object().ok_or("EXPECT.json must be an object")?;
    let mut out = std::collections::BTreeMap::new();
    for (file, val) in obj {
        let s = val.as_str().ok_or(format!("{file}: expectation must be a string"))?;
        let (check, measured) = match s.split_once('/') {
            Some((c, m)) => (c, Some(m)),
            None => (s, None),
        };
        if !CHECK_VERDICTS.contains(&check) {
            return Err(format!("{file}: unknown expectation {s}"));
        }
        if let Some(m) = measured {
            if check != "PASS" || !(m == "*" || MEASURE_VERDICTS.contains(&m)) {
                return Err(format!("{file}: bad measured expectation {s}"));
            }
        }
        out.insert(file.clone(), Expect { check: check.into(), measured: measured.map(String::from) });
    }
    Ok(out)
}

/// The text of the outcome (`PASS/TIED` for a measured control) and whether it meets `e`.
/// `measured` is the measured verdict or `NOISE_GATE_FAILED`; it is only looked at after a PASS.
pub fn judge(e: &Expect, check: &str, measured: Option<&str>) -> (String, bool) {
    let got = match (check, measured) {
        ("PASS", Some(m)) => format!("PASS/{m}"),
        _ => check.to_string(),
    };
    let ok = match (&e.measured, measured) {
        _ if check != e.check => false,
        (None, _) => true,
        (Some(want), Some(m)) => MEASURE_VERDICTS.contains(&m) && (want == "*" || want == m),
        (Some(_), None) => false,
    };
    (got, ok)
}

pub fn selftest_dir() -> PathBuf {
    match std::env::var_os("MSX_SELFTEST_DIR") {
        Some(d) => PathBuf::from(d),
        None => root().join("experiment/selftest"),
    }
}

#[derive(Clone, Debug)]
pub struct SelftestEntry {
    /// Sub-directory of the self-test directory (`vint`, `index_framing`, `reasm`).
    pub dir: String,
    pub file: String,
    /// The kernel the file is a candidate for (the file stem in `reasm/`).
    pub kernel: String,
    pub path: PathBuf,
    pub expect: Expect,
    pub reasm: bool,
}

/// Every candidate file (`.s`, `.rs`) in each sub-directory of `dir`, with its expectation. A file
/// without an entry, or an entry without a file, is an error.
pub fn selftest_entries(dir: &Path) -> Result<Vec<SelftestEntry>, String> {
    let mut subs: Vec<_> = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?.filter_map(|e| e.ok()).filter(|e| e.path().is_dir()).collect();
    subs.sort_by_key(|e| e.file_name());
    let mut out = Vec::new();
    for sub in subs {
        let dname = sub.file_name().to_string_lossy().into_owned();
        let ep = sub.path().join("EXPECT.json");
        let text = std::fs::read_to_string(&ep).map_err(|e| format!("{}: {e}", ep.display()))?;
        let mut expect = parse_expect(&text).map_err(|e| format!("{}: {e}", ep.display()))?;
        let mut files: Vec<String> = std::fs::read_dir(sub.path())
            .map_err(|e| e.to_string())?
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".s") || n.ends_with(".rs"))
            .collect();
        files.sort();
        let reasm = dname == "reasm";
        for f in files {
            let e = expect.remove(&f).ok_or(format!("{dname}/{f} has no entry in {}", ep.display()))?;
            let kname = if reasm { f.trim_end_matches(".s").to_string() } else { dname.clone() };
            if reasm && !f.ends_with(".s") {
                return Err(format!("{dname}/{f}: A.reasm candidates are assembly"));
            }
            if kernel(&kname).is_none() {
                return Err(format!("{dname}/{f}: unknown kernel {kname}"));
            }
            out.push(SelftestEntry { dir: dname.clone(), file: f.clone(), kernel: kname, path: sub.path().join(&f), expect: e, reasm });
        }
        if let Some(stale) = expect.keys().next() {
            return Err(format!("{} names {stale}, which does not exist", ep.display()));
        }
    }
    Ok(out)
}

/// Lint, build and check one file; for a measured control also the search-protocol measurement
/// against A.r3 at `THRESHOLD_FLOOR`. Returns the result object.
fn selftest_one(e: &SelftestEntry) -> Result<Value, String> {
    let arm = if e.file.ends_with(".rs") { Arm::Rust } else { Arm::Asm };
    let mut detail = serde_json::Map::new();
    let finish = |check: &str, measured: Option<&str>, detail: serde_json::Map<String, Value>| {
        let (got, ok) = judge(&e.expect, check, measured);
        let mut v = json!({ "kernel": e.kernel, "file": format!("{}/{}", e.dir, e.file), "expected": expected_text(&e.expect), "got": got, "ok": ok });
        for (k, x) in detail {
            v[k] = x;
        }
        v
    };
    let errs = lint_source(arm, &e.path);
    if !errs.is_empty() {
        detail.insert("lint_errors".into(), json!(errs));
        return Ok(finish("LINT_FAILED", None, detail));
    }
    let stem = e.file.rsplit_once('.').map_or(e.file.as_str(), |x| x.0);
    let out_dir = work().join("build/selftest").join(&e.dir).join(stem);
    let built = match arm {
        Arm::Asm => build_asm_candidate(&e.kernel, &e.path, &out_dir),
        Arm::Rust => build_rust_candidate(&e.kernel, &e.path, Rung::R3, &out_dir),
    };
    let so = match built {
        Ok(so) => so,
        Err(b) if b.stage == "setup" => return Err(b.to_string()),
        Err(b) if runtime_verdict_of_stage(b.stage).is_some() => {
            detail.insert("build_stage".into(), "train".into());
            detail.insert("build_stderr".into(), b.stderr.into());
            return Ok(finish(runtime_verdict_of_stage(b.stage).unwrap_or("OOB_OR_CRASH"), None, detail));
        }
        Err(b) => {
            detail.insert("build_stage".into(), b.stage.into());
            detail.insert("build_stderr".into(), b.stderr.into());
            return Ok(finish("BUILD_FAILED", None, detail));
        }
    };
    let errs = lint_object(arm, &e.kernel, &so);
    if !errs.is_empty() {
        detail.insert("lint_errors".into(), json!(errs));
        return Ok(finish("LINT_FAILED", None, detail));
    }
    let report = try_check(&e.kernel, &so, Phase::Search, None)?;
    detail.insert("cases_run".into(), report.cases_run.into());
    if report.verdict != "PASS" {
        detail.insert("first_failure".into(), serde_json::to_value(&report.first_failure).map_err(|x| x.to_string())?);
        return Ok(finish(&report.verdict, None, detail));
    }
    if e.reasm {
        let dst = reasm_path(&e.kernel);
        std::fs::create_dir_all(dst.parent().unwrap()).map_err(|x| x.to_string())?;
        std::fs::copy(&so, &dst).map_err(|x| format!("{}: {x}", dst.display()))?;
    }
    if e.expect.measured.is_none() {
        return Ok(finish("PASS", None, detail));
    }
    let (_, k) = traced_kernel(&[String::from("--kernel"), e.kernel.clone()])?;
    let cs = vec![contestant("A.r3", a_path(Rung::R3), k), contestant("cand", so.clone(), k)];
    detail.insert("threshold".into(), THRESHOLD_FLOOR.into());
    match run_protocol(&e.kernel, &cs, Protocol::Search, Split::Search) {
        Ok(res) => {
            let i = iv(&res, "cand", "A.r3");
            detail.insert("ratio_vs_A.r3".into(), serde_json::to_value(i).map_err(|x| x.to_string())?);
            detail.insert("protocol_sizes".into(), sizes(Protocol::Search, Some(&res)));
            let m = i.map(|i| verdict(i, THRESHOLD_FLOOR)).unwrap_or("INCONCLUSIVE");
            Ok(finish("PASS", Some(m), detail))
        }
        Err(x) if is_noise(&x) => {
            detail.insert("noise_detail".into(), x.into());
            Ok(finish("PASS", Some("NOISE_GATE_FAILED"), detail))
        }
        Err(x) => Err(x),
    }
}

fn expected_text(e: &Expect) -> String {
    match &e.measured {
        Some(m) => format!("{}/{m}", e.check),
        None => e.check.clone(),
    }
}

/// `msx selftest`: every sabotage and control file under `experiment/selftest/`. Consumes no agent
/// budget; appends one `selftest` record to the ledger.
pub fn selftest(_args: &[String]) -> Result<Value, String> {
    refuse_size_override()?;
    let entries = selftest_entries(&selftest_dir())?;
    let _host = host_lock()?;
    require_a(&[Rung::R1, Rung::R2, Rung::R3])?;
    let mut results = Vec::new();
    for e in &entries {
        eprintln!("msx: selftest {}/{}", e.dir, e.file);
        let r = selftest_one(e)?;
        eprintln!("msx:   expected {} got {} ok {}", r["expected"], r["got"], r["ok"]);
        results.push(r);
    }
    let pass = results.iter().all(|r| r["ok"] == true);
    let mut rec = json!({
        "kind": "selftest", "kernel": null, "arm": null, "pass": pass, "results": results,
        "noise_failure": false, "protocol_sizes": sizes(Protocol::Search, None),
    });
    let id = ledger::append(rec.clone())?;
    rec["id"] = id.into();
    Ok(rec)
}

// ---------------------------------------------------------------------------------------------
// report
// ---------------------------------------------------------------------------------------------

const CEILING_BUCKETS: [&str; 9] = ["own_decoding", "allocator", "memcpy", "decompression", "checksum", "sync", "other_cqlite", "bench_harness", "other"];
const CEILING_BENCHES: [&str; 3] = ["read", "partition_lookup", "decode"];
const COUNTER_ORDER: [&str; 8] = ["A.r1", "A.r2", "A.r3", "A.r3copy", "A.inline", "B.asm", "B.rust", "B.rust.r2"];

fn fnum(v: &Value, dec: usize) -> String {
    v.as_f64().map_or("–".to_string(), |x| format!("{x:.dec$}"))
}

/// Point and interval at the fewest decimals (3 to 5) at which the rounded `lo` and `hi` differ and
/// the rounded point lies between them. Returns `(point, "lo–hi")`.
fn fnum_iv(i: &Value) -> (String, String) {
    let (Some(p), Some(lo), Some(hi)) = (i["point"].as_f64(), i["lo"].as_f64(), i["hi"].as_f64()) else { return ("–".into(), "–".into()) };
    let mut dec = 3;
    while dec < 5 {
        // round to `dec` decimals and compare as numbers (not as strings)
        let r = |x: f64| format!("{x:.dec$}").parse::<f64>().unwrap_or(x);
        let (a, b, c) = (r(lo), r(hi), r(p));
        if a != b && a <= c && c <= b {
            break;
        }
        dec += 1;
    }
    (format!("{p:.dec$}"), format!("{lo:.dec$}–{hi:.dec$}"))
}

/// `0.901–0.923` from an `{lo, hi}` object.
fn fspan(i: &Value, dec: usize) -> String {
    match (i["lo"].as_f64(), i["hi"].as_f64()) {
        (Some(lo), Some(hi)) => format!("{lo:.dec$}–{hi:.dec$}"),
        _ => "–".to_string(),
    }
}

fn read_json(p: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()
}

fn ceiling_section(results: &Path, out: &mut String) {
    let mut rows = Vec::new();
    let mut commit = None;
    for b in CEILING_BENCHES {
        let rel = format!("profile/{b}.json");
        if let Some(j) = read_json(&results.join(&rel)) {
            commit = commit.or_else(|| j["cqlite_commit"].as_str().map(str::to_string));
            let cells: Vec<String> = CEILING_BUCKETS.iter().map(|k| j["buckets"][k].as_f64().map_or("–".into(), |x| format!("{:.1}", x * 100.0))).collect();
            rows.push(format!("| {b} | {} | {} | `{rel}` |", j["samples"].as_u64().map_or("–".into(), |n| n.to_string()), cells.join(" | ")));
        }
    }
    out.push_str("## 1. Ceiling (share of user-space cycle samples, percent)\n\n");
    if rows.is_empty() {
        out.push_str("no profile data (experiment/results/profile/*.json is missing)\n\n");
        return;
    }
    out.push_str(&format!("| bench | samples | {} | source |\n|---|---|{}---|\n", CEILING_BUCKETS.join(" | "), "---|".repeat(CEILING_BUCKETS.len())));
    for r in rows {
        out.push_str(&r);
        out.push('\n');
    }
    out.push_str(&format!("\nThe ceiling is the `own_decoding` column. CQLite commit {}. Rows end with their source file (they come from no ledger record).\n\n", commit.unwrap_or_else(|| "unknown".into())));
}

/// The row for `cand` vs `base`, from the final record's tool-issued verdicts.
fn verdict_row(fin: &Value, id: &str, cand: &str, base: &str) -> String {
    let found = fin["verdicts"].as_array().and_then(|a| a.iter().find(|p| p["cand"] == cand && p["base"] == base));
    let arm_status = |arm: &str| fin["arms"][arm]["status"].as_str().unwrap_or("NO_ENTRY").to_string();
    match found {
        Some(p) => {
            let (pt, span) = fnum_iv(&p["interval"]);
            format!("| {cand} vs {base} | {pt} | {span} | {} | {id} |", p["verdict"].as_str().unwrap_or("–"))
        }
        None => {
            let rust_in = fin["arms"]["rust"]["status"] == "ENTERED";
            let why = match (cand, base) {
                ("B.rust", _) | (_, "B.rust") if !rust_in => arm_status("rust"),
                ("B.asm", _) => arm_status("asm"),
                ("B.rust.r2", _) if rust_in => "OMITTED".into(),
                ("B.rust.r2", _) => arm_status("rust"),
                _ => "n/a".into(),
            };
            format!("| {cand} vs {base} | – | – | {why} | {id} |")
        }
    }
}

fn kernel_section(name: &str, fin: &Value, ledger: &[Value], out: &mut String) {
    let id = fin["id"].as_str().unwrap_or("unknown");
    out.push_str(&format!("### {name} (final {id})\n\n"));
    // consistency with the ledger: the file must be the latest final record
    let latest = ledger.iter().rev().find(|r| r["kind"] == "final" && r["kernel"] == name && r["status"] == "OK").and_then(|r| r["id"].as_str());
    match latest {
        Some(l) if l == id => {}
        Some(l) => out.push_str(&format!("WARNING: final/{name}.json is {id} but the latest OK final in the ledger is {l}.\n\n")),
        None => out.push_str(&format!("WARNING: the ledger holds no OK final record for {name}; final/{name}.json says {id}.\n\n")),
    }
    if let Some(rec) = ledger.iter().find(|r| r["id"] == id && r["kind"] == "final") {
        for f in ["status", "verdicts", "band", "nulls", "boundary_tax", "arms"] {
            if rec[f] != fin[f] {
                out.push_str(&format!("WARNING: final/{name}.json field `{f}` differs from ledger record {id}.\n\n"));
            }
        }
        if rec["measure"]["medians"] != fin["measure"]["medians"] {
            out.push_str(&format!("WARNING: final/{name}.json medians differ from ledger record {id}.\n\n"));
        }
    }
    out.push_str(&format!("Status: {}.", fin["status"].as_str().unwrap_or("unknown")));
    if let Some(rv) = fin["rustc_version"].as_str() {
        out.push_str(&format!(" Compiler: {rv}."));
    }
    out.push_str("\n\n| arm | status | attempt | cases checked | detail | ledger id |\n|---|---|---|---|---|---|\n");
    for arm in ["asm", "rust"] {
        let a = &fin["arms"][arm];
        let detail = a["reason"].as_str().map(str::to_string).or_else(|| a["r2_omitted"].as_str().map(|r| format!("B.rust.r2 omitted: {r}"))).unwrap_or_default();
        out.push_str(&format!(
            "| {arm} | {} | {} | {} | {detail} | {id} |\n",
            a["status"].as_str().unwrap_or("NO_ENTRY"),
            a["attempt"].as_u64().map_or("–".to_string(), |n| format!("attempt-{n:02}")),
            a["check"]["cases_run"].as_u64().map_or("–".to_string(), |n| n.to_string()),
        ));
    }
    out.push('\n');
    if fin["measure"].is_null() || fin["verdicts"].is_null() {
        out.push_str("No verdicts were issued by this final record.\n\n");
        return;
    }

    out.push_str("| comparison | ratio | 95% interval | verdict | ledger id |\n|---|---|---|---|---|\n");
    let mut rows = Vec::new();
    for cand in ["B.asm", "B.rust"] {
        for base in ["A.r1", "A.r2", "A.r3"] {
            rows.push((cand, base));
        }
    }
    rows.extend([("B.rust.r2", "A.r2"), ("B.asm", "B.rust"), ("A.r2", "A.r1"), ("A.r3", "A.r2")]);
    for (c, b) in rows {
        out.push_str(&verdict_row(fin, id, c, b));
        out.push('\n');
    }
    if let Some(nulls) = fin["nulls"].as_object() {
        for (k, i) in nulls {
            let label = k.replace('/', " vs ");
            out.push_str(&format!("| {label} (null) | {} | {} | NULL | {id} |\n", fnum(&i["point"], 5), fspan(i, 5)));
        }
    }
    let bt = &fin["boundary_tax"];
    if bt["interval"].is_object() {
        let (pt, span) = fnum_iv(&bt["interval"]);
        out.push_str(&format!("| {} vs {} (boundary tax) | {pt} | {span} | BOUNDARY | {id} |\n", bt["cand"].as_str().unwrap_or("A.r3"), bt["base"].as_str().unwrap_or("A.inline")));
    }
    let m = &fin["measure"];
    out.push_str(&format!(
        "\nNull band {} ({} of 1.0); T = {} (T = max(0.05, 2 x band)). Ratio is candidate / baseline cycles per op; below 1 the candidate is faster. {}\nIntervals are a bootstrap over sessions at one build placement; they do not include layout variation between binaries.\n",
        fnum(&fin["band"], 5),
        fin["band"].as_f64().map_or("–".into(), |b| format!("{:.3}%", b * 100.0)),
        fnum(&fin["threshold"], 3),
        if fin["nulls"].as_object().is_some_and(|o| o.contains_key("A.reasm/A.r3")) { "Nulls: A.r3copy vs A.r3 and A.reasm vs A.r3." } else { "No A.reasm null is present; the only null is A.r3copy vs A.r3." }
    ));
    out.push_str(&format!(
        "\nMeasured on the {} split, {} sessions x {} blocks, core {}, session reruns {}, inputs repeat: {}.{}\n\n",
        m["split"].as_str().unwrap_or("?"),
        m["sessions_used"],
        m["blocks_used"],
        m["core"],
        m["reruns"],
        if m["inputs_repeat"] == true { "yes (small-trace mode)" } else { "no" },
        m["rejected_sessions"].as_array().map_or(String::new(), |a| format!(" Rejected sessions: {}.", a.len()))
    ));

    out.push_str("| contestant | cycles/op | instructions/op | branch misses/op | ns/op | ledger id |\n|---|---|---|---|---|---|\n");
    let meds = m["medians"].as_object();
    let mut names: Vec<String> = COUNTER_ORDER.iter().filter(|n| meds.is_some_and(|o| o.contains_key(**n))).map(|s| s.to_string()).collect();
    if let Some(o) = meds {
        names.extend(o.keys().filter(|k| !COUNTER_ORDER.contains(&k.as_str())).cloned());
    }
    for n in names {
        let c = &m["medians"][&n];
        out.push_str(&format!(
            "| {n} | {} | {} | {} | {} | {id} |\n",
            fnum(&c["cycles_per_op"], 2),
            fnum(&c["instructions_per_op"], 2),
            fnum(&c["branch_misses_per_op"], 4),
            fnum(&c["ns_per_op"], 3)
        ));
    }
    out.push('\n');
}

fn attempts_section(kernels: &[String], ledger: &[Value], out: &mut String) {
    out.push_str("## 3. Attempts history\n\nSearch-phase numbers are provisional and are not results.\n\n");
    for k in kernels {
        for arm in ["asm", "rust"] {
            let recs: Vec<&Value> = ledger.iter().filter(|r| (r["kind"] == "attempt" || r["kind"] == "refused") && r["kernel"] == k.as_str() && r["arm"] == arm).collect();
            out.push_str(&format!("### {k} / {arm}\n\n"));
            if recs.is_empty() {
                out.push_str("no submissions in the ledger\n\n");
                continue;
            }
            out.push_str("| submission | counted | verdict | provisional (search phase) ratio vs A.r3, or failure kind | ledger id |\n|---|---|---|---|---|\n");
            for r in recs {
                let verdict_s = r["verdict"].as_str().unwrap_or("–");
                let pair = r["measure"]["pairs"].as_array().and_then(|a| a.iter().find(|p| p["cand"] == "cand" && p["base"] == "A.r3"));
                let last = match pair {
                    Some(p) => {
                        let (pt, span) = fnum_iv(&p["interval"]);
                        format!("{pt} ({span})")
                    }
                    None => verdict_s.to_string(),
                };
                out.push_str(&format!(
                    "| {} | {} | {verdict_s} | {last} | {} |\n",
                    r["submission"].as_u64().map_or("–".to_string(), |n| n.to_string()),
                    r["counted_attempt"].as_u64().map_or("free".to_string(), |n| n.to_string()),
                    r["id"].as_str().unwrap_or("–"),
                ));
            }
            out.push('\n');
        }
    }
}

fn incidents_section(results: &Path, ledger: &[Value], out: &mut String) {
    out.push_str("## 4. Incidents\n\nFrom the ledger (noise-gate failures and refusals):\n\n");
    let mut any = false;
    for r in ledger {
        let noisy = r["noise_failure"] == true || r["kind"] == "refused" || (r["pass"] == false && (r["kind"] == "null" || r["kind"] == "selftest"));
        if noisy {
            any = true;
            let detail = r["noise_detail"].as_str().or_else(|| r["reason"].as_str()).unwrap_or("");
            out.push_str(&format!(
                "- {} ({}, {}, {}): {detail}\n",
                r["id"].as_str().unwrap_or("?"),
                r["kind"].as_str().unwrap_or("?"),
                r["kernel"].as_str().unwrap_or("-"),
                r["ts"].as_str().unwrap_or("?")
            ));
        }
    }
    if !any {
        out.push_str("none\n");
    }
    out.push_str("\nFrom `experiment/results/incidents.md`:\n\n");
    match std::fs::read_to_string(results.join("incidents.md")) {
        Ok(t) => {
            out.push_str(t.trim_end());
            out.push_str("\n\n");
        }
        Err(_) => out.push_str("no incidents file\n\n"),
    }
}

fn provenance_section(results: &Path, ledger: &[Value], out: &mut String) {
    out.push_str("## 5. Provenance\n\n");
    let ids: Vec<&str> = ledger.iter().filter_map(|r| r["id"].as_str()).collect();
    let (Some(first), Some(last)) = (ids.first(), ids.last()) else {
        out.push_str("The ledger is empty.\n");
        return;
    };
    out.push_str(&format!("- Ledger id range: {first} to {last} ({} records).\n", ids.len()));
    let ts: Vec<&str> = ledger.iter().filter_map(|r| r["ts"].as_str()).collect();
    out.push_str(&format!("- Dates: {} to {}.\n", ts.first().unwrap_or(&"?"), ts.last().unwrap_or(&"?")));
    // tool hashes, with the ids that cite each
    let mut tools: Vec<(String, Vec<&str>)> = Vec::new();
    for r in ledger {
        if let (Some(h), Some(id)) = (r["tool_sha256"].as_str(), r["id"].as_str()) {
            match tools.iter_mut().find(|(t, _)| t == h) {
                Some((_, v)) => v.push(id),
                None => tools.push((h.to_string(), vec![id])),
            }
        }
    }
    for (h, v) in &tools {
        out.push_str(&format!("- Tool sha256 {h}: {} records ({} to {}).\n", v.len(), v[0], v[v.len() - 1]));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Ok(h) = sha256_hex(&exe) {
            out.push_str(&format!("- This report was produced by a separate build of the tool, sha256 {h} (it only reads files).\n"));
        }
    }
    // A hashes: each distinct set, with the ids that recorded it
    let mut sets: Vec<(String, Vec<&str>)> = Vec::new();
    for r in ledger {
        if r["a_sha256"].is_object() {
            let s = r["a_sha256"].to_string();
            let id = r["id"].as_str().unwrap_or("?");
            match sets.iter_mut().find(|(t, _)| *t == s) {
                Some((_, v)) => v.push(id),
                None => sets.push((s, vec![id])),
            }
        }
    }
    for (s, v) in &sets {
        let j: Value = serde_json::from_str(s).unwrap_or(Value::Null);
        let parts: Vec<String> = j.as_object().map(|o| o.iter().map(|(k, h)| format!("A.{k} {}", h.as_str().unwrap_or("?"))).collect()).unwrap_or_default();
        out.push_str(&format!("- A hashes ({} records, {} to {}): {}.\n", v.len(), v[0], v[v.len() - 1], parts.join(", ")));
    }
    let commit = CEILING_BENCHES.iter().find_map(|b| read_json(&results.join(format!("profile/{b}.json"))).and_then(|j| j["cqlite_commit"].as_str().map(str::to_string)));
    out.push_str(&format!("- CQLite commit: {}.\n", commit.unwrap_or_else(|| "unknown (no profile data)".into())));
    let host = &ledger[ledger.len() - 1]["host"];
    out.push_str(&format!(
        "- Host: {}, Linux {}, governor {}.\n",
        host["cpu_model"].as_str().unwrap_or("?"),
        host["kernel_release"].as_str().unwrap_or("?"),
        host["governor"].as_str().unwrap_or("?")
    ));
    let dirty: Vec<&str> = ledger.iter().filter(|r| r["git_dirty"] == true).filter_map(|r| r["id"].as_str()).collect();
    out.push_str(&format!("- Records written with `git_dirty` true: {}.\n", if dirty.is_empty() { "none".to_string() } else { dirty.join(", ") }));
}

/// The Markdown report. Reads the ledger (`MSX_LEDGER` or the default path) and the `final/`,
/// `profile/` and `incidents.md` files beside it. Takes no lock and writes nothing.
pub fn report() -> Result<String, String> {
    let ledger_file = ledger::ledger_path();
    let results = ledger_file.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
    let recs = ledger::read_all();
    let mut out = String::from("# msx report\n\n");
    let lines = std::fs::read_to_string(&ledger_file).map_or(0, |t| t.lines().filter(|l| !l.trim().is_empty()).count());
    if lines != recs.len() {
        out.push_str(&format!("WARNING: {} ledger line(s) are not valid JSON and were skipped.\n\n", lines - recs.len()));
    }
    ceiling_section(&results, &mut out);

    let mut finals: Vec<(String, Value)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(results.join("final")) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "json") {
                if let (Some(stem), Some(j)) = (p.file_stem().and_then(|s| s.to_str()), read_json(&p)) {
                    finals.push((stem.to_string(), j));
                }
            }
        }
    }
    finals.sort_by(|a, b| a.0.cmp(&b.0));
    out.push_str("## 2. Verdicts per kernel\n\n");
    if finals.is_empty() {
        out.push_str("no final results (experiment/results/final/*.json is missing)\n\n");
    }
    for (name, j) in &finals {
        kernel_section(name, j, &recs, &mut out);
    }
    let mut kernels: Vec<String> = finals.iter().map(|(n, _)| n.clone()).collect();
    for r in &recs {
        if r["kind"] == "attempt" {
            if let Some(k) = r["kernel"].as_str() {
                if !kernels.iter().any(|x| x == k) {
                    kernels.push(k.to_string());
                }
            }
        }
    }
    kernels.sort();
    attempts_section(&kernels, &recs, &mut out);
    incidents_section(&results, &recs, &mut out);
    provenance_section(&results, &recs, &mut out);
    Ok(out)
}
