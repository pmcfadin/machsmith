use msx_harness::commands;
use msx_harness::check::{child_main, phase_params, try_check, Phase};
use msx_harness::build::{a_path, build_a, disassemble, sha256_hex, Rung};
use msx_harness::loader::Lib;
use msx_harness::measure::{run_protocol, session_main, Contestant, Protocol, Split};
use msx_harness::stats::{threshold, verdict};
use msx_harness::spec::{kernel, kernels_with_traces, load_segment, manifest_segments, KernelSpec};
use std::path::PathBuf;

fn arg_after(args: &[String], flag: &str) -> Option<String> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned()
}

fn rungs_build() -> Result<(), String> {
    let mut built = serde_json::Map::new();
    for (name, rung) in [("r1", Rung::R1), ("r2", Rung::R2), ("r3", Rung::R3), ("r3copy", Rung::R3Copy)] {
        eprintln!("msx: building A.{name}");
        let so = build_a(rung).map_err(|e| e.to_string())?;
        built.insert(name.into(), sha256_hex(&so).map_err(|e| e.to_string())?.into());
    }
    let mut differs = serde_json::Map::new();
    for k in kernels_with_traces() {
        let r2 = disassemble(&a_path(Rung::R2), k.symbol())?;
        let r3 = disassemble(&a_path(Rung::R3), k.symbol())?;
        differs.insert(k.name().into(), (r2 != r3).into());
    }
    println!("{}", serde_json::json!({ "built": built, "r3_differs_from_r2": differs }));
    Ok(())
}

/// PGO training driver. Must return from `main` normally so the profile runtime writes its file.
fn train(args: &[String]) -> Result<(), String> {
    let so = PathBuf::from(arg_after(args, "--so").ok_or("--so required")?);
    let which = arg_after(args, "--kernels").ok_or("--kernels required")?;
    let lib = Lib::open(&so)?;
    let ks: Vec<&'static dyn KernelSpec> = if which == "all" {
        kernels_with_traces()
    } else {
        which
            .split(',')
            .map(|n| kernel(n).ok_or(format!("unknown kernel {n}")))
            .collect::<Result<_, _>>()?
    };
    for k in ks {
        let f = lib.sym(k.symbol()).ok_or(format!("{} not exported by {}", k.symbol(), so.display()))?;
        let chain = lib.sym(k.chain_symbol());
        let segs = manifest_segments(k.name(), Some("search"))?;
        if segs.is_empty() {
            return Err(format!("{}: no search segments", k.name()));
        }
        let mut total_ops = 0u64;
        for seg in &segs {
            let buf = load_segment(&seg.path).map_err(|e| format!("{}: {e}", seg.path.display()))?;
            for _pass in 0..3 {
                let (ops, _sum) = unsafe { k.run_chain(f, &buf) };
                total_ops += ops;
                if let Some(c) = chain {
                    let mut sum = 0u64;
                    let cf: unsafe extern "C" fn(*const u8, usize, *mut u64) -> u64 = unsafe { std::mem::transmute(c) };
                    total_ops += unsafe { cf(buf.as_ptr(), buf.len(), &mut sum) };
                }
            }
        }
        eprintln!("msx: trained {} on {} segments ({total_ops} ops)", k.name(), segs.len());
    }
    Ok(())
}

fn check_cmd(args: &[String]) -> Result<(), String> {
    let kernel = arg_after(args, "--kernel").ok_or("--kernel required")?;
    let so = PathBuf::from(arg_after(args, "--so").ok_or("--so required")?);
    let phase = match arg_after(args, "--phase").ok_or("--phase required")?.as_str() {
        "search" => Phase::Search,
        "final" => Phase::Final,
        o => return Err(format!("bad --phase {o} (search|final)")),
    };
    let seed_from = arg_after(args, "--seed-from").map(PathBuf::from);
    phase_params(phase, seed_from.as_deref())?;
    let report = try_check(&kernel, &so, phase, seed_from.as_deref())?;
    println!("{}", serde_json::to_string(&report).map_err(|e| e.to_string())?);
    Ok(())
}

/// `msx measure --kernel K --protocol search|final --split search|holdout --contestants name=path[:chain],...
///  [--null-band B]`. `:chain` after a path calls `msx_<k>_chain` directly (A.inline).
fn measure_cmd(args: &[String]) -> Result<(), String> {
    let kname = arg_after(args, "--kernel").ok_or("--kernel required")?;
    let k = kernel(&kname).ok_or(format!("unknown kernel {kname}"))?;
    let protocol = match arg_after(args, "--protocol").ok_or("--protocol required")?.as_str() {
        "search" => Protocol::Search,
        "final" => Protocol::Final,
        o => return Err(format!("bad --protocol {o} (search|final)")),
    };
    let split = match arg_after(args, "--split").ok_or("--split required")?.as_str() {
        "search" => Split::Search,
        "holdout" => Split::Holdout,
        o => return Err(format!("bad --split {o} (search|holdout)")),
    };
    let mut cs = Vec::new();
    for item in arg_after(args, "--contestants").ok_or("--contestants required")?.split(',') {
        let (name, rest) = item.split_once('=').ok_or(format!("bad contestant {item} (name=path[:chain])"))?;
        let (path, is_chain) = match rest.strip_suffix(":chain") {
            Some(p) => (p, true),
            None => (rest, false),
        };
        let symbol = if is_chain { k.chain_symbol() } else { k.symbol() };
        cs.push(Contestant { name: name.into(), so: PathBuf::from(path), symbol: symbol.into(), is_chain });
    }
    let band: f64 = match arg_after(args, "--null-band") {
        Some(b) => b.parse().map_err(|e| format!("--null-band: {e}"))?,
        None => 0.0,
    };
    let t = threshold(band);
    match run_protocol(&kname, &cs, protocol, split) {
        Ok(res) => {
            let verdicts: Vec<_> = res
                .pairs
                .iter()
                .map(|p| serde_json::json!({ "cand": p.cand, "base": p.base, "interval": p.interval, "verdict": p.interval.map(|iv| verdict(iv, t)).unwrap_or("INCONCLUSIVE") }))
                .collect();
            let mut v = serde_json::to_value(&res).map_err(|e| e.to_string())?;
            v["threshold"] = t.into();
            v["verdicts"] = verdicts.into();
            println!("{v}");
            Ok(())
        }
        Err(e) if e.starts_with("NOISE_GATE_FAILED") => {
            println!("{}", serde_json::json!({ "verdict": "NOISE_GATE_FAILED", "detail": e }));
            Ok(())
        }
        Err(e) => Err(e),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let r = match args.get(1).map(String::as_str) {
        Some("rungs") if args.get(2).map(String::as_str) == Some("build") => rungs_build(),
        Some("__train") => train(&args),
        Some("measure") => measure_cmd(&args),
        Some("__session") => session_main(&args),
        Some("check") => check_cmd(&args),
        Some("attempt") => commands::attempt(&args).map(|v| println!("{v}")),
        Some("null") => commands::null(&args).map(|v| println!("{v}")),
        Some("selftest") => commands::selftest(&args).map(|v| println!("{}", serde_json::json!({ "pass": v["pass"], "results": v["results"], "id": v["id"] }))),
        Some("final") => commands::final_exam(&args).map(|v| println!("{v}")),
        Some("__check-child") => child_main(&args),
        _ => Err("usage: msx rungs build | msx attempt --kernel K --arm asm|rust --file PATH | msx null --kernel K | msx selftest | msx final --kernel K [--rerun-reason TEXT] | msx check --kernel K --so PATH --phase search|final [--seed-from FILE] | msx measure --kernel K --protocol search|final --split search|holdout --contestants name=path[:chain],.. | msx __train --so PATH --kernels all|k,..".into()),
    };
    if let Err(e) = r {
        eprintln!("msx: {e}");
        std::process::exit(2);
    }
}
