use r2_referee::criterion::read_home;
use std::path::PathBuf;

fn fx(n: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(n)
}

#[test]
fn reads_two_cases() {
    let m = read_home(&fx("criterion_home")).unwrap();
    let c = &m["decode/type_int"];
    assert_eq!(c.median_ns, 10.0);
    assert_eq!(c.samples, 3);
    assert!((c.mean_ns - 31.0 / 3.0).abs() < 1e-9);
    assert_eq!(c.mad_ns, 0.0); // per-iter 10,11,10: deviations 0,1,0
    assert_eq!(m["decode/type_text"].median_ns, 102.5);
    assert_eq!(m["decode/type_text"].mad_ns, 2.5);
}

#[test]
fn full_id_not_dirname() {
    let m = read_home(&fx("criterion_home")).unwrap();
    assert!(m.contains_key("read/get_partition_big (bti)"));
    assert!(!m.contains_key("read/get_partition_big_bti_"));
}

#[test]
fn ignores_base_and_change_dirs() {
    let m = read_home(&fx("criterion_home")).unwrap();
    assert_eq!(m.len(), 3);
    assert_eq!(m["decode/type_int"].samples, 3);
}

#[test]
fn empty_home_is_error() {
    let e = read_home(&fx("criterion_empty")).unwrap_err();
    assert!(e.contains("criterion_empty"), "{e}");
    let e = read_home(&fx("does_not_exist")).unwrap_err();
    assert!(e.contains("does_not_exist"), "{e}");
}
