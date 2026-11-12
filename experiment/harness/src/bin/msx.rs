use msx_harness::check::{child_main, phase_params, try_check, Phase};
use msx_harness::build::{a_path, build_a, disassemble, sha256_hex, Rung};
use msx_harness::loader::Lib;
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

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let r = match args.get(1).map(String::as_str) {
        Some("rungs") if args.get(2).map(String::as_str) == Some("build") => rungs_build(),
        Some("__train") => train(&args),
        Some("check") => check_cmd(&args),
        Some("__check-child") => child_main(&args),
        _ => Err("usage: msx rungs build | msx check --kernel K --so PATH --phase search|final [--seed-from FILE] | msx __train --so PATH --kernels all|k,..".into()),
    };
    if let Err(e) = r {
        eprintln!("msx: {e}");
        std::process::exit(2);
    }
}
