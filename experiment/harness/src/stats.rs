//! Session ratios, the session bootstrap, the null band, the threshold and the verdict table
//! (spec section 7.2).

use crate::measure::{Sample, BOOTSTRAP_RESAMPLES, THRESHOLD_FLOOR};
use rand_chacha::ChaCha20Rng;
use rand_core::{RngCore, SeedableRng};

pub const BOOTSTRAP_SEED: u64 = 0x6D7378;
/// Fewer valid blocks than this and a session has no ratio.
pub const MIN_VALID_BLOCKS: usize = 10;

#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct Interval {
    pub point: f64,
    pub lo: f64,
    pub hi: f64,
}

pub fn median(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = v.len();
    if n == 0 {
        return f64::NAN;
    }
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

/// One session's ratio: over blocks in which neither contestant has a discarded sample, the median
/// of `(cand cycles/op) / (base cycles/op)`. `None` with fewer than 10 such blocks.
pub fn session_ratio(samples: &[Sample], cand: &str, base: &str) -> Option<f64> {
    use std::collections::BTreeMap;
    // (session, block) -> (cand sample, base sample)
    let mut blocks: BTreeMap<(u32, u32), (Option<&Sample>, Option<&Sample>)> = BTreeMap::new();
    for s in samples {
        let e = blocks.entry((s.session, s.block)).or_default();
        if s.contestant == cand && e.0.is_none() {
            e.0 = Some(s);
        } else if s.contestant == base && e.1.is_none() {
            e.1 = Some(s);
        }
    }
    let mut ratios: Vec<f64> = blocks
        .values()
        .filter_map(|(c, b)| {
            let (c, b) = ((*c)?, (*b)?);
            if c.discard.is_some() || b.discard.is_some() || c.ops == 0 || b.ops == 0 || b.cycles == 0 {
                return None;
            }
            Some((c.cycles as f64 / c.ops as f64) / (b.cycles as f64 / b.ops as f64))
        })
        .collect();
    if ratios.len() < MIN_VALID_BLOCKS {
        return None;
    }
    Some(median(&mut ratios))
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    let pos = p * (sorted.len() - 1) as f64;
    let (i, f) = (pos.floor() as usize, pos - pos.floor());
    if i + 1 < sorted.len() {
        sorted[i] * (1.0 - f) + sorted[i + 1] * f
    } else {
        sorted[i]
    }
}

/// Geometric mean of session ratios with a 95% percentile-bootstrap interval over sessions.
pub fn bootstrap_ci(session_ratios: &[f64]) -> Interval {
    let n = session_ratios.len();
    if n == 0 {
        return Interval { point: f64::NAN, lo: f64::NAN, hi: f64::NAN };
    }
    let logs: Vec<f64> = session_ratios.iter().map(|r| r.ln()).collect();
    let point = (logs.iter().sum::<f64>() / n as f64).exp();
    let mut rng = ChaCha20Rng::seed_from_u64(BOOTSTRAP_SEED);
    let mut means = Vec::with_capacity(BOOTSTRAP_RESAMPLES);
    for _ in 0..BOOTSTRAP_RESAMPLES {
        let mut sum = 0.0;
        for _ in 0..n {
            sum += logs[(rng.next_u64() % n as u64) as usize];
        }
        means.push((sum / n as f64).exp());
    }
    means.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Interval { point, lo: percentile(&means, 0.025), hi: percentile(&means, 0.975) }
}

/// Largest distance of any null bound from 1.0.
pub fn null_band(nulls: &[Interval]) -> f64 {
    nulls.iter().map(|iv| (1.0 - iv.lo).max(iv.hi - 1.0)).fold(0.0, f64::max)
}

pub fn threshold(band: f64) -> f64 {
    THRESHOLD_FLOOR.max(2.0 * band)
}

/// Spec 7.2. The ratio is candidate / baseline cycles per op.
pub fn verdict(iv: Interval, t: f64) -> &'static str {
    if iv.hi < 1.0 - t {
        "WON"
    } else if iv.lo > 1.0 + t {
        "LOST"
    } else if iv.lo >= 1.0 - t && iv.hi <= 1.0 + t {
        "TIED"
    } else {
        "INCONCLUSIVE"
    }
}
