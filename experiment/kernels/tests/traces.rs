//! Recorded traces: manifests match segments, no duplicates, trailing garbage rejected,
//! the split rule is deterministic, and the index_framing corpus has the expected size.

use msx_kernels::traces::*;
use std::collections::HashSet;
use std::path::PathBuf;

fn traces_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../traces")
}

fn manifests() -> Vec<(PathBuf, Manifest)> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(traces_root()) else { return out };
    for e in rd.flatten() {
        let p = e.path().join("MANIFEST.json");
        if p.exists() {
            let m: Manifest = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
            out.push((e.path(), m));
        }
    }
    out
}

/// 8-byte aligned copy, as the bti chain requires.
fn aligned(bytes: &[u8]) -> (Vec<u64>, usize) {
    let mut v = vec![0u64; bytes.len().div_ceil(8).max(1)];
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), v.as_mut_ptr() as *mut u8, bytes.len()) };
    (v, bytes.len())
}

fn chain_ops(kernel: &str, bytes: &[u8]) -> u64 {
    let (buf, len) = aligned(bytes);
    let mut cs = 0u64;
    let p = buf.as_ptr() as *const u8;
    unsafe {
        match kernel {
            "vint" => msx_kernels::vint::msx_vint_chain(p, len, &mut cs),
            "index_framing" => msx_kernels::index_framing::msx_index_framing_chain(p, len, &mut cs),
            _ => msx_kernels::bti_sparse_find::msx_bti_sparse_find_chain(p, len, &mut cs),
        }
    }
}

#[test]
fn manifest_matches_segments() {
    let ms = manifests();
    assert!(!ms.is_empty(), "no trace manifests under experiment/traces");
    for (dir, m) in ms {
        let (mut sops, mut hops, mut sb, mut hb) = (0, 0, 0, 0);
        for s in &m.segments {
            let bytes = std::fs::read(dir.join(&s.file)).unwrap_or_else(|e| panic!("{}: {e}", s.file));
            assert_eq!(hex(&sha256(&bytes)), s.sha256, "{} sha256", s.file);
            assert_eq!(bytes.len() as u64, s.bytes, "{} bytes", s.file);
            let ops = walk(&m.kernel, &bytes).unwrap_or_else(|e| panic!("{}: {e}", s.file));
            assert_eq!(ops, s.ops, "{} ops", s.file);
            assert_eq!(chain_ops(&m.kernel, &bytes), s.ops, "{} chain ops", s.file);
            let th = m.split_threshold.unwrap_or(DEFAULT_SPLIT_THRESHOLD);
            assert_eq!(split_for_with(&bytes, th), s.split, "{} split", s.file);
            assert!(s.file.starts_with(s.split.dir()));
            match s.split {
                Split::Search => (sops, sb) = (sops + s.ops, sb + s.bytes),
                Split::Holdout => (hops, hb) = (hops + s.ops, hb + s.bytes),
            }
        }
        assert_eq!((sops, hops, sb, hb),
            (m.totals.search_ops, m.totals.holdout_ops, m.totals.search_bytes, m.totals.holdout_bytes));
        assert_eq!(m.small_trace.search, is_small_trace(sops));
        assert_eq!(m.small_trace.holdout, is_small_trace(hops));
    }
}

#[test]
fn no_duplicate_content() {
    for (_, m) in manifests() {
        let mut seen = HashSet::new();
        for s in &m.segments {
            assert!(seen.insert(s.sha256.clone()), "{}: duplicate content {}", m.kernel, s.file);
        }
    }
}

#[test]
fn stream_with_trailing_garbage_is_rejected() {
    // entry: [u16 BE key_len][key][vuint data_offset][vuint promoted_len=0]
    let mut v = Vec::new();
    for k in [&b"ab"[..], &b"cde"[..], &b"f"[..]] {
        v.extend_from_slice(&(k.len() as u16).to_be_bytes());
        v.extend_from_slice(k);
        v.extend_from_slice(&[0x05, 0x00]);
    }
    assert_eq!(walk("index_framing", &v), Ok(3));
    v.push(0xFF);
    let err = walk("index_framing", &v).expect_err("trailing garbage must be rejected");
    assert!(err.contains("before end") || err.contains("error"), "{err}");
}

#[test]
fn walks_reject_leftovers_for_every_kernel() {
    assert!(walk("vint", &[0x01, 0x80]).is_err());
    assert_eq!(walk("vint", &[0x01, 0x80, 0x80]), Ok(2));
    let mut rec = Vec::new();
    rec.extend_from_slice(&1u32.to_le_bytes());
    rec.extend_from_slice(&7u32.to_le_bytes());
    rec.extend_from_slice(&[7, 0, 0, 0, 0, 0, 0, 0]);
    rec.extend_from_slice(&9u64.to_le_bytes());
    assert_eq!(walk("bti_sparse_find", &rec), Ok(1));
    rec.push(0);
    assert!(walk("bti_sparse_find", &rec).is_err());
}

#[test]
fn split_rule_is_deterministic() {
    assert_eq!(split_for(b"abc"), split_for(b"abc"));
    // sha256("abc") = ba7816bf...: first byte 0xba = 186 >= 77 -> search
    assert_eq!(split_for(b"abc"), Split::Search);
    // sha256("") = e3b0c442...: 0xe3 = 227 -> search; sha256("a") = ca978112...: 0xca -> search;
    // sha256("b") = 3e23e816...: 0x3e = 62 < 77 -> holdout
    assert_eq!(split_for(b"b"), Split::Holdout);
    assert_eq!(split_for_with(b"b", 62), Split::Search);
    assert_eq!(split_for_with(b"b", 63), Split::Holdout);
}

#[test]
fn index_framing_counts() {
    let ms = manifests();
    let (_, m) = ms.iter().find(|(_, m)| m.kernel == "index_framing").expect("index_framing manifest");
    // 152 BIG Index.db files: every one is a segment, a content duplicate of one, or excluded.
    let accounted = m.segments.len() as u64 + m.duplicates_dropped + m.excluded.len() as u64;
    assert!(accounted >= 150, "{accounted} files accounted for");
    assert!(m.segments.len() >= 50, "{} segments", m.segments.len());
    let total = m.totals.search_ops + m.totals.holdout_ops;
    assert!((4_000..=5_500).contains(&total), "{total} ops");
}
