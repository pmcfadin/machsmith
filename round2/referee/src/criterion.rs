//! Reads a Criterion 0.5.1 output tree (`CRITERION_HOME`) into per-case statistics.

use crate::stats::median;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct CaseStat {
    pub id: String,
    pub median_ns: f64,
    pub mean_ns: f64,
    /// Plain median absolute deviation of the per-iteration times (no 1.4826 scale factor).
    pub mad_ns: f64,
    pub samples: usize,
}

#[derive(Deserialize)]
struct Benchmark {
    full_id: String,
}

#[derive(Deserialize)]
struct Sample {
    iters: Vec<f64>,
    times: Vec<f64>,
}

fn find_new_dirs(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let rd = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for ent in rd {
        let p = ent.map_err(|e| format!("{}: {e}", dir.display()))?.path();
        if !p.is_dir() {
            continue;
        }
        if p.file_name().and_then(|n| n.to_str()) == Some("new") {
            if p.join("benchmark.json").is_file() && p.join("sample.json").is_file() {
                out.push(p);
            }
        } else {
            find_new_dirs(&p, out)?;
        }
    }
    Ok(())
}

fn read_json<T: for<'de> Deserialize<'de>>(p: &Path) -> Result<T, String> {
    let s = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
    serde_json::from_str(&s).map_err(|e| format!("{}: {e}", p.display()))
}

/// Reads every `<home>/**/new/benchmark.json` + `sample.json`; keys by `full_id`.
/// Only `new/` directories are read; `base/` and `change/` are ignored.
pub fn read_home(home: &Path) -> Result<BTreeMap<String, CaseStat>, String> {
    let mut dirs = Vec::new();
    find_new_dirs(home, &mut dirs)?;
    dirs.sort();
    let mut out = BTreeMap::new();
    for d in dirs {
        let bm: Benchmark = read_json(&d.join("benchmark.json"))?;
        let sp = d.join("sample.json");
        let s: Sample = read_json(&sp)?;
        if s.iters.len() != s.times.len() || s.iters.is_empty() {
            return Err(format!("{}: iters/times length mismatch or empty", sp.display()));
        }
        let mut per: Vec<f64> = Vec::with_capacity(s.iters.len());
        for (t, i) in s.times.iter().zip(&s.iters) {
            if !(*i > 0.0) || !t.is_finite() {
                return Err(format!("{}: invalid iters/times entry", sp.display()));
            }
            per.push(t / i);
        }
        let mean_ns = per.iter().sum::<f64>() / per.len() as f64;
        let med = median(&mut per.clone());
        let mut dev: Vec<f64> = per.iter().map(|x| (x - med).abs()).collect();
        let mad_ns = median(&mut dev);
        if out.contains_key(&bm.full_id) {
            return Err(format!("{}: duplicate full_id {}", d.display(), bm.full_id));
        }
        out.insert(
            bm.full_id.clone(),
            CaseStat { id: bm.full_id, median_ns: med, mean_ns, mad_ns, samples: per.len() },
        );
    }
    if out.is_empty() {
        return Err(format!("{}: no Criterion cases found", home.display()));
    }
    Ok(out)
}
