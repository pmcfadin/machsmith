//! Builds `experiment/traces/<kernel>/{search,holdout}/*.seg` and `MANIFEST.json`
//! from raw capture output (vint, bti_sparse_find) and the datasets' Index.db files.
//!
//! msx-trace-manifest --raw work/traces-raw --datasets <root> --out experiment/traces

use msx_kernels::traces::*;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

const CQLITE_COMMIT: &str = "2e5b7a4cf6abe6f51b7677ecb7199e2cab087c0e";

struct Cand {
    origin: String,
    content: Vec<u8>,
    sha: [u8; 32],
    ops: u64,
    stats: Stats,
}

fn walk_dir(dir: &Path, out: &mut Vec<PathBuf>, pred: &dyn Fn(&Path) -> bool) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk_dir(&p, out, pred);
        } else if pred(&p) {
            out.push(p);
        }
    }
}

fn quantiles(v: &mut Vec<u64>) -> Value {
    if v.is_empty() {
        return json!({});
    }
    v.sort_unstable();
    let q = |p: f64| v[((v.len() - 1) as f64 * p).round() as usize];
    json!({"count": v.len(), "min": v[0], "p10": q(0.1), "p25": q(0.25), "p50": q(0.5),
           "p75": q(0.75), "p90": q(0.9), "p99": q(0.99), "max": v[v.len() - 1]})
}

fn histogram(kernel: &str, st: &Stats) -> Value {
    match kernel {
        "vint" => {
            let mut m = serde_json::Map::new();
            for l in 1..=9 {
                m.insert(l.to_string(), json!(st.vint_len[l]));
            }
            json!({"ops_by_encoded_length": m})
        }
        "index_framing" => json!({
            "key_len": quantiles(&mut st.key_lens.clone()),
            "promoted_len": quantiles(&mut st.promoted_lens.clone()),
        }),
        _ => {
            let mut counts = std::collections::BTreeMap::new();
            for n in &st.bti_n {
                *counts.entry(*n).or_insert(0u64) += 1;
            }
            let ops = st.bti_n.len() as u64;
            json!({
                "n": quantiles(&mut st.bti_n.clone()),
                "n_counts": counts.iter().map(|(k, v)| (k.to_string(), json!(v))).collect::<serde_json::Map<_, _>>(),
                "hits": st.bti_hits,
                "hit_rate": if ops == 0 { 0.0 } else { st.bti_hits as f64 / ops as f64 },
            })
        }
    }
}

fn merge(a: &mut Stats, b: &Stats) {
    for i in 0..10 {
        a.vint_len[i] += b.vint_len[i];
    }
    a.key_lens.extend(&b.key_lens);
    a.promoted_lens.extend(&b.promoted_lens);
    a.bti_n.extend(&b.bti_n);
    a.bti_hits += b.bti_hits;
}

fn collect(kernel: &str, raw: &Path, datasets: &Path) -> (Vec<Cand>, Vec<Excluded>) {
    // (origin, path) pairs
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    if kernel == "index_framing" {
        let mut v = Vec::new();
        walk_dir(datasets, &mut v, &|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.ends_with("-big-Index.db"))
        });
        for p in v {
            let rel = p.strip_prefix(datasets).unwrap().to_string_lossy().into_owned();
            files.push((rel, p));
        }
    } else {
        let mut v = Vec::new();
        let prefix = format!("{kernel}.");
        walk_dir(raw, &mut v, &|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(&prefix) && n.ends_with(".bin"))
        });
        for p in v {
            let origin = std::fs::read_to_string(p.parent().unwrap().join("ORIGIN"))
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|_| p.parent().unwrap().to_string_lossy().into_owned());
            files.push((origin, p));
        }
    }
    files.sort();
    let (mut cands, mut excluded) = (Vec::new(), Vec::new());
    for (origin, path) in files {
        let content = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        if content.is_empty() {
            continue;
        }
        let mut stats = Stats::default();
        match walk_stats(kernel, &content, Some(&mut stats)) {
            Ok(ops) => cands.push(Cand { origin, sha: sha256(&content), content, ops, stats }),
            Err(reason) => excluded.push(Excluded { origin, reason }),
        }
    }
    cands.sort_by(|a, b| (&a.origin, a.sha).cmp(&(&b.origin, b.sha)));
    (cands, excluded)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (mut raw, mut datasets, mut out) = (None, None, None);
    while let Some(a) = args.next() {
        let v = args.next().unwrap_or_else(|| panic!("{a} needs a value"));
        match a.as_str() {
            "--raw" => raw = Some(PathBuf::from(v)),
            "--datasets" => datasets = Some(PathBuf::from(v)),
            "--out" => out = Some(PathBuf::from(v)),
            _ => panic!("unknown argument {a}"),
        }
    }
    let (raw, datasets, out) = (raw.expect("--raw"), datasets.expect("--datasets"), out.expect("--out"));

    for kernel in KERNELS {
        let (cands, excluded) = collect(kernel, &raw, &datasets);
        // dedupe by content, keeping the first in (origin, sha) order
        let mut seen = HashSet::new();
        let mut dups = 0u64;
        let cands: Vec<Cand> = cands
            .into_iter()
            .filter(|c| {
                let fresh = seen.insert(c.sha);
                dups += !fresh as u64;
                fresh
            })
            .collect();
        let total_ops: u64 = cands.iter().map(|c| c.ops).sum();
        if kernel == "bti_sparse_find" && total_ops < 2_000 {
            // size rule: fewer than 2,000 recorded operations drops the kernel
            let _ = std::fs::remove_dir_all(out.join(kernel));
            eprintln!("{kernel}: DROPPED, only {total_ops} recorded calls in the local data");
            continue;
        }
        let holdout_at = |t: u8| -> u64 {
            cands.iter().filter(|c| c.sha[0] < t).map(|c| c.ops).sum()
        };
        let mut threshold = DEFAULT_SPLIT_THRESHOLD;
        let h = holdout_at(threshold);
        let mut changed = false;
        if h == 0 || (h as f64) < 0.10 * total_ops as f64 {
            // smallest threshold byte that puts at least 20% of operations in holdout
            threshold = (1..=255u16)
                .find(|t| holdout_at(*t as u8) as f64 >= 0.20 * total_ops as f64)
                .map(|t| t as u8)
                .unwrap_or_else(|| panic!("{kernel}: no threshold gives 20% holdout"));
            changed = true;
        }

        let kdir = out.join(kernel);
        let _ = std::fs::remove_dir_all(&kdir);
        for s in [Split::Search, Split::Holdout] {
            std::fs::create_dir_all(kdir.join(s.dir())).unwrap();
        }
        let mut segments = Vec::new();
        let mut totals = Totals::default();
        let (mut hs, mut hh) = (Stats::default(), Stats::default());
        for (i, c) in cands.iter().enumerate() {
            let split = split_for_with(&c.content, threshold);
            let file = format!("{}/{:04}.seg", split.dir(), i);
            std::fs::write(kdir.join(&file), &c.content).unwrap();
            match split {
                Split::Search => {
                    totals.search_ops += c.ops;
                    totals.search_bytes += c.content.len() as u64;
                    merge(&mut hs, &c.stats);
                }
                Split::Holdout => {
                    totals.holdout_ops += c.ops;
                    totals.holdout_bytes += c.content.len() as u64;
                    merge(&mut hh, &c.stats);
                }
            }
            segments.push(SegmentEntry {
                file,
                origin: c.origin.clone(),
                split,
                sha256: hex(&c.sha),
                bytes: c.content.len() as u64,
                ops: c.ops,
            });
        }
        let capture_commands = if kernel == "index_framing" {
            vec![format!(
                "find {} -name '*-big-Index.db' (no instrumentation); msx-trace-manifest --datasets {} --out experiment/traces",
                datasets.display(),
                datasets.display()
            )]
        } else {
            vec![
                "experiment/trace-capture/capture.sh (see experiment/trace-capture/README.md for every command line)".to_string(),
                "msx-trace-manifest --raw work/traces-raw --datasets <datasets root> --out experiment/traces".to_string(),
            ]
        };
        let manifest = Manifest {
            kernel: kernel.to_string(),
            cqlite_commit: CQLITE_COMMIT.to_string(),
            capture_commands,
            format: if kernel == "bti_sparse_find" { "records" } else { "stream" }.to_string(),
            split_threshold: changed.then_some(threshold),
            segments,
            excluded,
            duplicates_dropped: dups,
            small_trace: SmallTrace {
                search: is_small_trace(totals.search_ops),
                holdout: is_small_trace(totals.holdout_ops),
            },
            totals,
            histograms: json!({"search": histogram(kernel, &hs), "holdout": histogram(kernel, &hh)}),
        };
        let mut text = serde_json::to_string_pretty(&manifest).unwrap();
        text.push('\n');
        std::fs::write(kdir.join("MANIFEST.json"), text).unwrap();
        eprintln!(
            "{kernel}: {} segments, search {} ops / holdout {} ops, excluded {}, duplicates {}{}",
            manifest.segments.len(),
            manifest.totals.search_ops,
            manifest.totals.holdout_ops,
            manifest.excluded.len(),
            manifest.duplicates_dropped,
            if changed { format!(", split_threshold {threshold}") } else { String::new() }
        );
    }
}
