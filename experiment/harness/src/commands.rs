//! The `attempt`, `null` and `final` commands. They hold an exclusive `flock` on `work/msx.lock`
//! from start to finish, so only one build-check-measure sequence runs on the host at a time.
//!
//! Failure policy: a *tool* error (bad usage, dirty tree, missing rung build, a candidate that
//! cannot even be loaded, a measurement that fails for a reason other than the noise gate) is an
//! `Err` that `main` turns into exit code 2 and writes no ledger record. A *verdict* (lint, build,
//! check, budget, noise gate) is recorded and printed with exit code 0.

use crate::build::{a_path, build_asm_candidate, build_rust_candidate, root, sha256_hex, work, Rung};
use crate::check::{try_check, CheckReport, Phase};
use crate::ledger::{self, budget, COUNTED_ATTEMPTS, MAX_SUBMISSIONS};
use crate::lint::{lint_object, lint_source, Arm};
use crate::measure::{
    run_protocol, size_override_active, Contestant, Protocol, RunResult, Split, FINAL_BLOCKS, FINAL_SESSIONS, NULL_BAND_MAX, SEARCH_BLOCKS,
    SEARCH_SESSIONS,
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
    refuse_size_override()?;
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
        Err(e) => return free_failure("BUILD_FAILED", vec![], json!({ "stage": e.stage, "stderr": e.stderr })),
    };
    let errs = lint_object(arm, &kname, &so);
    if !errs.is_empty() {
        return free_failure("LINT_FAILED", errs, Value::Null);
    }

    // 6. correctness (this attempt counts from here on). A tool error here writes no record.
    require_a(&[Rung::R1, Rung::R2, Rung::R3])?;
    let counted_no = b.counted + 1;
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
        Err(e) => Err(e),
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
    refuse_size_override()?;
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
                "measure": null, "protocol_sizes": sizes(Protocol::Final, None),
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

pub fn final_exam(args: &[String]) -> Result<Value, String> {
    let (kname, k) = traced_kernel(args)?;
    let rerun_reason = arg_after(args, "--rerun-reason");
    refuse_size_override()?;
    let _host = host_lock()?;

    // 1. one measured final per kernel unless a rerun is justified
    let ledger_before = ledger::read_all();
    let measured_before = ledger_before.iter().any(|r| r["kind"] == "final" && r["kernel"] == kname.as_str() && !r["measure"].is_null());
    if measured_before && rerun_reason.as_deref().map_or(true, |r| r.trim().is_empty()) {
        return Err(format!(
            "the ledger already holds a measured final for {kname}; a rerun is legitimate only after a tool failure or NOISE_GATE_FAILED, and needs --rerun-reason TEXT"
        ));
    }

    // 2-4. finalists
    let mut arms = serde_json::Map::new();
    let mut finalists: Vec<(Arm, Finalist)> = Vec::new();
    for arm in [Arm::Asm, Arm::Rust] {
        let arm_s = arm_name(arm);
        let dir = root().join("experiment/candidates").join(&kname).join(arm_s);
        let no_entry = |why: String| json!({ "status": "NO_ENTRY", "reason": why });
        let Ok(text) = std::fs::read_to_string(dir.join("FINAL.md")) else {
            arms.insert(arm_s.into(), no_entry("FINAL.md is missing".into()));
            continue;
        };
        let Some(n) = parse_final_md(&text) else {
            arms.insert(arm_s.into(), no_entry("FINAL.md is not `FINAL: attempt-NN`".into()));
            continue;
        };
        let Some(rec) = ledger_before
            .iter()
            .rev()
            .find(|r| r["kind"] == "attempt" && r["kernel"] == kname.as_str() && r["arm"] == arm_s && r["submission"] == n)
            .filter(|r| r["verdict"] == "PASS")
        else {
            arms.insert(arm_s.into(), no_entry(format!("attempt-{n:02} has no PASS record in the ledger")));
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
        let so = match arm {
            Arm::Asm => build_asm_candidate(&kname, &src, &out_dir),
            Arm::Rust => build_rust_candidate(&kname, &src, Rung::R3, &out_dir),
        }
        .map_err(|e| format!("rebuilding the finalist {}: {e}", cand_rel))?;
        let report = try_check(&kname, &so, Phase::Final, Some(&src))?;
        entry["check"] = serde_json::to_value(&report).map_err(|e| e.to_string())?;
        if report.verdict != "PASS" {
            arms.insert(arm_s.into(), disq(entry, format!("final check: {}", report.verdict)));
            continue;
        }
        let r2 = if arm == Arm::Rust {
            Some(build_rust_candidate(&kname, &src, Rung::R2, &out_dir).map_err(|e| format!("building the r2 finalist: {e}"))?)
        } else {
            None
        };
        entry["status"] = "ENTERED".into();
        arms.insert(arm_s.into(), entry);
        finalists.push((arm, Finalist { so, r2 }));
    }

    let mut rec = json!({
        "kind": "final", "kernel": kname, "arm": null, "arms": Value::Object(arms), "rerun_reason": rerun_reason,
        "measure": null, "nulls": null, "band": null, "threshold": null, "verdicts": null, "boundary_tax": null,
        "noise_failure": false, "protocol_sizes": null, "status": "NO_FINALISTS",
    });

    if !finalists.is_empty() {
        require_a(&[Rung::R1, Rung::R2, Rung::R3, Rung::R3Copy])?;
        // 5. contestants
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
                    rec["status"] = "NOISE_GATE_FAILED".into();
                    rec["noise_detail"] = format!("null band {band:.4} > {NULL_BAND_MAX}").into();
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
            }
            Err(e) if is_noise(&e) => {
                rec["status"] = "NOISE_GATE_FAILED".into();
                rec["noise_failure"] = Value::Bool(true);
                rec["noise_detail"] = e.into();
                rec["protocol_sizes"] = sizes(Protocol::Final, None);
            }
            Err(e) => return Err(e),
        }
    }

    let id = ledger::append(rec.clone())?;
    rec["id"] = id.into();
    write_json(&root().join("experiment/results/final").join(format!("{kname}.json")), &rec)?;
    if let Some(o) = rec.as_object_mut() {
        o.remove("measure");
    }
    Ok(rec)
}
