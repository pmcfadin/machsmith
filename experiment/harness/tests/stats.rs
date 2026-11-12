use msx_harness::measure::Sample;
use msx_harness::stats::*;

fn close(a: f64, b: f64, eps: f64) -> bool {
    (a - b).abs() <= eps
}

#[test]
fn bootstrap_of_constant() {
    let iv = bootstrap_ci(&[0.9; 30]);
    assert!(close(iv.point, 0.9, 1e-12), "{iv:?}");
    assert!(close(iv.lo, 0.9, 1e-12), "{iv:?}");
    assert!(close(iv.hi, 0.9, 1e-12), "{iv:?}");
}

#[test]
fn bootstrap_is_deterministic() {
    let r: Vec<f64> = (0..30).map(|i| 0.9 + 0.004 * (i as f64) + if i % 2 == 0 { 0.01 } else { -0.01 }).collect();
    let a = bootstrap_ci(&r);
    let b = bootstrap_ci(&r);
    assert_eq!((a.point, a.lo, a.hi), (b.point, b.lo, b.hi));
    assert!(a.lo < a.point && a.point < a.hi, "{a:?}");
}

#[test]
fn verdict_table() {
    let t = 0.05;
    let iv = |lo, hi| Interval { point: (lo + hi) / 2.0, lo, hi };
    assert_eq!(verdict(iv(0.90, 0.94), t), "WON");
    assert_eq!(verdict(iv(0.96, 1.03), t), "TIED");
    assert_eq!(verdict(iv(1.06, 1.20), t), "LOST");
    assert_eq!(verdict(iv(0.93, 0.97), t), "INCONCLUSIVE");
    assert_eq!(verdict(iv(0.94, 1.06), t), "INCONCLUSIVE");
}

#[test]
fn threshold_rule() {
    assert_eq!(threshold(0.01), 0.05);
    assert_eq!(threshold(0.04), 0.08);
}

#[test]
fn null_band_rule() {
    let iv = |lo, hi| Interval { point: 1.0, lo, hi };
    let band = null_band(&[iv(0.99, 1.02), iv(0.97, 1.00)]);
    assert!(close(band, 0.03, 1e-12), "{band}");
}

fn synth(ratios: &[f64], discard_blocks: &[u32]) -> Vec<Sample> {
    use msx_harness::measure::Discard;
    let mut v = Vec::new();
    for (b, r) in ratios.iter().enumerate() {
        let b = b as u32;
        for (name, cyc) in [("base", 1000u64), ("cand", (1000.0 * r) as u64)] {
            v.push(Sample {
                session: 0,
                block: b,
                chunk: 0,
                contestant: name.into(),
                ops: 100,
                cycles: cyc,
                instructions: 5000,
                branch_misses: 1,
                wall_ns: 300,
                discard: if discard_blocks.contains(&b) { Some(Discard::Preempted) } else { None },
            });
        }
    }
    v
}

#[test]
fn session_ratio_median() {
    let mut ratios = vec![0.5];
    ratios.extend([0.9; 9]);
    ratios.push(2.0);
    let s = synth(&ratios, &[]);
    let r = session_ratio(&s, "cand", "base").unwrap();
    assert!(close(r, 0.9, 1e-9), "{r}");
    // only 9 valid blocks
    let s = synth(&ratios, &[0, 10]);
    assert!(session_ratio(&s, "cand", "base").is_none());
}
