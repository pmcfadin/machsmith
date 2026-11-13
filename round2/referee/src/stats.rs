//! The session bootstrap, set intervals, the null band, the threshold and the verdict table
//! (spec section 7.2). Ported from round 1's harness; bootstrap and verdict logic are unchanged.

use rand_chacha::ChaCha20Rng;
use rand_core::{RngCore, SeedableRng};
use std::collections::BTreeMap;

pub const BOOTSTRAP_SEED: u64 = 0x6D7378;
pub const BOOTSTRAP_RESAMPLES: usize = 10_000;
pub const THRESHOLD_FLOOR: f64 = 0.05;

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct Interval {
    pub point: f64,
    pub lo: f64,
    pub hi: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Verdict {
    Won,
    Lost,
    Tied,
    Inconclusive,
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

/// Geometric mean; NaN for an empty slice.
pub fn geomean(v: &[f64]) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    (v.iter().map(|r| r.ln()).sum::<f64>() / v.len() as f64).exp()
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

/// Largest distance of an interval bound from 1.0: `max(1 - lo, hi - 1)`.
pub fn band_of(iv: &Interval) -> f64 {
    (1.0 - iv.lo).max(iv.hi - 1.0)
}

pub fn threshold(band: f64) -> f64 {
    THRESHOLD_FLOOR.max(2.0 * band)
}

/// Spec 7.2. The ratio is candidate / baseline.
pub fn verdict(iv: Interval, t: f64) -> Verdict {
    if iv.hi < 1.0 - t {
        Verdict::Won
    } else if iv.lo > 1.0 + t {
        Verdict::Lost
    } else if iv.lo >= 1.0 - t && iv.hi <= 1.0 + t {
        Verdict::Tied
    } else {
        Verdict::Inconclusive
    }
}

/// `sessions[s][case]` = cand/base ratio for that session. Returns the bootstrap interval of the
/// per-session geometric mean over `cases`. Errors if any session lacks any case.
pub fn set_interval(sessions: &[BTreeMap<String, f64>], cases: &[String]) -> Result<Interval, String> {
    let mut per_session = Vec::with_capacity(sessions.len());
    for (s, m) in sessions.iter().enumerate() {
        let mut ratios = Vec::with_capacity(cases.len());
        for c in cases {
            match m.get(c) {
                Some(r) => ratios.push(*r),
                None => return Err(format!("session {s} lacks case {c}")),
            }
        }
        per_session.push(geomean(&ratios));
    }
    Ok(bootstrap_ci(&per_session))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand_chacha::ChaCha20Rng;
    use rand_core::{RngCore, SeedableRng};
    use std::collections::BTreeMap;

    // Frozen oracle: byte-identical copy of round 1's `percentile` and `bootstrap_ci`
    // (experiment/harness/src/stats.rs). Ruling R3: this duplication is intended.
    fn round1_percentile(sorted: &[f64], p: f64) -> f64 {
        let pos = p * (sorted.len() - 1) as f64;
        let (i, f) = (pos.floor() as usize, pos - pos.floor());
        if i + 1 < sorted.len() {
            sorted[i] * (1.0 - f) + sorted[i + 1] * f
        } else {
            sorted[i]
        }
    }

    fn round1_bootstrap_ci(session_ratios: &[f64]) -> Interval {
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
        Interval { point, lo: round1_percentile(&means, 0.025), hi: round1_percentile(&means, 0.975) }
    }

    #[test]
    fn bootstrap_matches_round1() {
        let ratios = [0.95, 0.97, 0.96, 0.98, 0.94, 0.96, 0.97, 0.95, 0.96, 0.99];
        let a = bootstrap_ci(&ratios);
        let b = round1_bootstrap_ci(&ratios);
        assert_eq!(a.point, b.point);
        assert_eq!(a.lo, b.lo);
        assert_eq!(a.hi, b.hi);
    }

    #[test]
    fn verdict_table() {
        let t = 0.05;
        let iv = |point, lo, hi| Interval { point, lo, hi };
        assert_eq!(verdict(iv(0.90, 0.88, 0.94), t), Verdict::Won);
        assert_eq!(verdict(iv(1.10, 1.06, 1.12), t), Verdict::Lost);
        assert_eq!(verdict(iv(1.0, 0.96, 1.04), t), Verdict::Tied);
        assert_eq!(verdict(iv(0.95, 0.93, 0.97), t), Verdict::Inconclusive);
        assert_eq!(verdict(iv(0.95, 0.949, 1.0), t), Verdict::Inconclusive);
    }

    #[test]
    fn threshold_floor() {
        assert_eq!(threshold(0.01), 0.05);
        assert_eq!(threshold(0.04), 0.08);
    }

    #[test]
    fn geomean_basic() {
        assert!((geomean(&[0.81, 1.0]) - 0.9).abs() < 1e-12);
        assert!((geomean(&[2.0, 8.0]) - 4.0).abs() < 1e-12);
        assert!(geomean(&[]).is_nan());
    }

    #[test]
    fn band_of_is_max_distance() {
        let iv = Interval { point: 1.0, lo: 0.97, hi: 1.02 };
        assert!((band_of(&iv) - 0.03).abs() < 1e-12);
        let iv = Interval { point: 1.0, lo: 0.99, hi: 1.06 };
        assert!((band_of(&iv) - 0.06).abs() < 1e-12);
    }

    fn sessions(n: usize) -> Vec<BTreeMap<String, f64>> {
        (0..n)
            .map(|_| [("a".to_string(), 0.81), ("b".to_string(), 1.0)].into_iter().collect())
            .collect()
    }

    #[test]
    fn set_interval_geomean() {
        let cases = vec!["a".to_string(), "b".to_string()];
        let iv = set_interval(&sessions(10), &cases).unwrap();
        assert!((iv.point - 0.9).abs() < 1e-12);
    }

    #[test]
    fn set_interval_missing_case_errors() {
        let cases = vec!["a".to_string(), "b".to_string()];
        let mut s = sessions(10);
        s[3].remove("b");
        let e = set_interval(&s, &cases).unwrap_err();
        assert!(e.contains("b"), "{e}");
    }

    #[test]
    fn verdict_serializes_upper() {
        assert_eq!(serde_json::to_string(&Verdict::Won).unwrap(), "\"WON\"");
        assert_eq!(serde_json::to_string(&Verdict::Inconclusive).unwrap(), "\"INCONCLUSIVE\"");
    }
}
