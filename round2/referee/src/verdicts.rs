//! Interval, null-band and verdict computation over stored session records (spec 6.2, 6.3, 6.5,
//! 6.6), and the markdown report. Everything here reads the session JSON exactly as it goes to
//! the ledger, so a verdict can always be recomputed from its record.

use crate::stats::{band_of, bootstrap_ci, set_interval, threshold, verdict, Interval};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

/// perf-gate.json's per-case regression wall, reported as an extra column only.
pub const PERF_GATE: f64 = 1.10;

/// Per valid (unrejected) session: case id -> ratio.
pub fn ratio_maps(sessions: &Value) -> Vec<BTreeMap<String, f64>> {
    let empty = Vec::new();
    sessions
        .as_array()
        .unwrap_or(&empty)
        .iter()
        .filter(|s| s["rejection"].is_null())
        .map(|s| {
            s["cases"]
                .as_object()
                .map(|m| m.iter().filter_map(|(k, v)| Some((k.clone(), v["ratio"].as_f64()?))).collect())
                .unwrap_or_default()
        })
        .collect()
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
}

/// The null band (spec 6.3) for a target set, from a null record's per-session per-case ratios:
/// (`b` over `targets` if any, `b` over the null's whole workload).
pub fn null_band(null_record: &Value, workload: &str, targets: &[String]) -> Result<(Option<f64>, f64), String> {
    let id = null_record["id"].as_str().unwrap_or("?");
    if null_record["workload"].as_str() != Some(workload) {
        return Err(format!("null {id} is for workload {}, not {workload}", null_record["workload"]));
    }
    let cases = strings(&null_record["cases"]);
    if cases.is_empty() {
        return Err(format!("null {id} has no case list"));
    }
    if let Some(t) = targets.iter().find(|t| !cases.contains(t)) {
        return Err(format!("target {t} is not a case of null {id}"));
    }
    let sessions = ratio_maps(&null_record["sessions"]);
    if sessions.is_empty() {
        return Err(format!("null {id} has no valid sessions"));
    }
    let bt = if targets.is_empty() { None } else { Some(band_of(&set_interval(&sessions, targets)?)) };
    let bw = band_of(&set_interval(&sessions, &cases)?);
    Ok((bt, bw))
}

/// `T = threshold(max(b_targets, b_workload))`.
pub fn t_of(bt: Option<f64>, bw: f64) -> f64 {
    threshold(bt.map_or(bw, |b| b.max(bw)))
}

pub fn case_intervals(sessions: &[BTreeMap<String, f64>], cases: &[String]) -> Result<BTreeMap<String, Interval>, String> {
    let mut out = BTreeMap::new();
    for c in cases {
        let r: Vec<f64> = sessions
            .iter()
            .enumerate()
            .map(|(i, s)| s.get(c).copied().ok_or_else(|| format!("session {i} lacks case {c}")))
            .collect::<Result<_, _>>()?;
        out.insert(c.clone(), bootstrap_ci(&r));
    }
    Ok(out)
}

/// Fields for a null record: per-case and workload intervals, its workload band and T.
pub fn null_fields(sessions: &[BTreeMap<String, f64>], cases: &[String]) -> Result<Map<String, Value>, String> {
    let w = set_interval(sessions, cases)?;
    let bw = band_of(&w);
    let mut m = Map::new();
    m.insert("intervals".into(), json!({"cases": case_intervals(sessions, cases)?, "workload": w}));
    m.insert("b_workload".into(), json!(bw));
    m.insert("T_workload".into(), json!(threshold(bw)));
    Ok(m)
}

/// Fields for a verdict run: b, T, the target and workload verdicts, the headline `verdict`
/// (targets if any, else workload), and per-case results with the REGRESSION and perf-gate flags.
pub fn verdict_fields(
    sessions: &[BTreeMap<String, f64>],
    cases: &[String],
    targets: &[String],
    bt: Option<f64>,
    bw: f64,
) -> Result<Map<String, Value>, String> {
    let t = t_of(bt, bw);
    let w = set_interval(sessions, cases)?;
    let wv = json!({"interval": w, "verdict": verdict(w, t)});
    let tv = if targets.is_empty() {
        Value::Null
    } else {
        let iv = set_interval(sessions, targets)?;
        json!({"interval": iv, "verdict": verdict(iv, t)})
    };
    let headline = if tv.is_null() { wv["verdict"].clone() } else { tv["verdict"].clone() };
    let mut results = Map::new();
    let mut regressions = Vec::new();
    for (c, iv) in case_intervals(sessions, cases)? {
        let reg = iv.lo > 1.0 + t;
        if reg {
            regressions.push(c.clone());
        }
        results.insert(
            c,
            json!({"point": iv.point, "lo": iv.lo, "hi": iv.hi, "REGRESSION": reg, "perf_gate_10pct": iv.point > PERF_GATE}),
        );
    }
    let mut m = Map::new();
    m.insert("b_targets".into(), json!(bt));
    m.insert("b_workload".into(), json!(bw));
    m.insert("T".into(), json!(t));
    m.insert("verdicts".into(), json!({"targets": tv, "workload": wv}));
    m.insert("verdict".into(), headline);
    m.insert("results".into(), Value::Object(results));
    m.insert("regressions".into(), json!(regressions));
    Ok(m)
}

fn num(v: &Value) -> String {
    v.as_f64().map_or("-".into(), |x| format!("{x:.4}"))
}

fn iv_text(v: &Value) -> String {
    if v.is_null() {
        return "-".into();
    }
    format!("{} [{}, {}]", num(&v["point"]), num(&v["lo"]), num(&v["hi"]))
}

/// A short markdown summary of one record.
pub fn report(r: &Value) -> String {
    let s = |k: &str| r[k].as_str().unwrap_or("-").to_string();
    let mut o = format!("# {} {} — {}\n\n", s("id"), s("kind"), s("status"));
    o += &format!("- workload: {}\n- at: {}\n", s("workload"), s("at"));
    if r["test_mode"] == true {
        o += "- **test mode** (not a result)\n";
    }
    if r["provisional"] == true {
        o += "- **provisional** (screening run; never a reported result)\n";
    }
    if let Some(reason) = r["reason"].as_str() {
        o += &format!("- reason: {reason}\n");
    }
    o += &format!(
        "- sessions: {} valid of {} requested, {} rejected\n",
        r["sessions_completed"], r["sessions_requested"], r["sessions_rejected"]
    );
    let txt = |v: &Value| v.as_str().map_or(v.to_string(), String::from);
    o += &format!("- base sha256: {}\n- cand sha256: {}\n", txt(&r["base"]["sha256"]), txt(&r["cand"]["sha256"]));
    if !r["null_ref"].is_null() {
        o += &format!("- null: {}\n", txt(&r["null_ref"]));
    }
    o += &format!("- b_targets: {}, b_workload: {}, T: {}\n", num(&r["b_targets"]), num(&r["b_workload"]), num(&r["T"]));
    let v = &r["verdicts"];
    if v.is_object() {
        for k in ["targets", "workload"] {
            if !v[k].is_null() {
                o += &format!("- {k} verdict: **{}** {}\n", v[k]["verdict"].as_str().unwrap_or("-"), iv_text(&v[k]["interval"]));
            }
        }
    }
    if let Some(res) = r["results"].as_object() {
        o += "\n| case | ratio | lo | hi | REGRESSION | perf_gate_10pct |\n|---|---|---|---|---|---|\n";
        for (c, x) in res {
            let flag = |k: &str| if x[k] == true { k.to_string() } else { String::new() };
            o += &format!("| {c} | {} | {} | {} | {} | {} |\n", num(&x["point"]), num(&x["lo"]), num(&x["hi"]), flag("REGRESSION"), flag("perf_gate_10pct"));
        }
    } else if let Some(cs) = r["intervals"]["cases"].as_object() {
        o += &format!("- workload interval: {}\n", iv_text(&r["intervals"]["workload"]));
        o += "\n| case | ratio | lo | hi |\n|---|---|---|---|\n";
        for (c, x) in cs {
            o += &format!("| {c} | {} | {} | {} |\n", num(&x["point"]), num(&x["lo"]), num(&x["hi"]));
        }
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sessions(n: usize, a: f64, b: f64) -> Vec<BTreeMap<String, f64>> {
        (0..n).map(|i| BTreeMap::from([("a".into(), a + i as f64 * 1e-4), ("b".into(), b)])).collect()
    }

    #[test]
    fn regression_flag_uses_lo_above_one_plus_t() {
        let cases = vec!["a".to_string(), "b".to_string()];
        let m = verdict_fields(&sessions(8, 1.0, 1.06), &cases, &[], None, 0.01).unwrap();
        assert_eq!(m["T"], 0.05);
        assert_eq!(m["results"]["b"]["REGRESSION"], true);
        assert_eq!(m["results"]["b"]["perf_gate_10pct"], false);
        assert_eq!(m["results"]["a"]["REGRESSION"], false);
        assert_eq!(m["verdicts"]["targets"], Value::Null);
    }

    #[test]
    fn t_uses_larger_band() {
        assert_eq!(t_of(Some(0.04), 0.01), 0.08);
        assert_eq!(t_of(None, 0.03), 0.06);
        assert_eq!(t_of(Some(0.001), 0.001), 0.05);
    }

    #[test]
    fn ratio_maps_skip_rejected() {
        let s = json!([
            {"rejection": null, "cases": {"a": {"ratio": 1.0}}},
            {"rejection": {"Load": 4.0}, "cases": {"a": {"ratio": 9.0}}}
        ]);
        let m = ratio_maps(&s);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0]["a"], 1.0);
    }
}
