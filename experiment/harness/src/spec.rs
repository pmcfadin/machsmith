//! The `KernelSpec` abstraction, the kernel registry, aligned segment loading and
//! trace-manifest helpers shared by every later task.

use crate::tramp::AbiViolation;
use rand_chacha::ChaCha8Rng;
use rand_core::{RngCore, SeedableRng};
use std::path::{Path, PathBuf};

pub type RawFn = *const core::ffi::c_void;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize)]
#[repr(C)]
pub struct OutRec {
    pub ret: u64,
    pub out: [u64; 3],
    pub out_written: u64, // 0 or 1
}

pub trait KernelSpec: Sync {
    fn name(&self) -> &'static str;
    fn symbol(&self) -> &'static str;
    fn chain_symbol(&self) -> &'static str;
    /// Call the kernel once through the ABI trampoline. `input` is exactly the bytes it may read.
    unsafe fn run_case(&self, f: RawFn, input: *const u8, len: usize, aux: u64) -> Result<OutRec, AbiViolation>;
    /// Drive the kernel over one trace chunk as CQLite would. Returns (ops, checksum of outputs).
    unsafe fn run_chain(&self, f: RawFn, chunk: &[u8]) -> (u64, u64);
    /// Split a segment into chunks of at least `min_ops` operations, using the reference to find boundaries.
    unsafe fn chunks<'a>(&self, reference: RawFn, seg: &'a [u8], min_ops: u64) -> Vec<&'a [u8]>;
    /// Enumerate every individual call in a segment as (input bytes, aux).
    unsafe fn trace_cases(&self, reference: RawFn, seg: &[u8], sink: &mut dyn FnMut(&[u8], u64));
    /// Generated structural edge cases, deterministic.
    fn edge_cases(&self, sink: &mut dyn FnMut(&[u8], u64));
    /// `n` seeded random cases.
    fn random_cases(&self, seed: u64, n: u64, sink: &mut dyn FnMut(&[u8], u64));
}

static VINT: crate::spec_vint::VintSpec = crate::spec_vint::VintSpec;
static INDEX_FRAMING: crate::spec_index_framing::IndexFramingSpec = crate::spec_index_framing::IndexFramingSpec;
static BTI: crate::spec_bti_sparse_find::BtiSparseFindSpec = crate::spec_bti_sparse_find::BtiSparseFindSpec;
static ALL: [&'static dyn KernelSpec; 3] = [&VINT, &INDEX_FRAMING, &BTI];

pub fn kernel(name: &str) -> Option<&'static dyn KernelSpec> {
    ALL.iter().copied().find(|k| k.name() == name)
}

pub fn all_kernels() -> &'static [&'static dyn KernelSpec] {
    &ALL
}

/// Kernels with recorded traces (`experiment/traces/<k>/MANIFEST.json` exists).
/// `bti_sparse_find` has none: it was dropped after recording zero calls.
pub fn kernels_with_traces() -> Vec<&'static dyn KernelSpec> {
    all_kernels()
        .iter()
        .copied()
        .filter(|k| traces_dir().join(k.name()).join("MANIFEST.json").is_file())
        .collect()
}

/// `experiment/traces/`.
pub fn traces_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("traces")
}

/// A byte buffer whose start is 8-byte aligned (backed by `Vec<u64>`).
pub struct AlignedBuf {
    words: Vec<u64>,
    len: usize,
}

impl AlignedBuf {
    pub fn from_bytes(bytes: &[u8]) -> AlignedBuf {
        let mut words = vec![0u64; bytes.len().div_ceil(8)];
        unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), words.as_mut_ptr() as *mut u8, bytes.len()) };
        AlignedBuf { words, len: bytes.len() }
    }

    pub fn as_slice(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self.words.as_ptr() as *const u8, self.len) }
    }
}

impl core::ops::Deref for AlignedBuf {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        self.as_slice()
    }
}

/// Load a segment file into an 8-byte-aligned buffer.
pub fn load_segment(path: &Path) -> std::io::Result<AlignedBuf> {
    Ok(AlignedBuf::from_bytes(&std::fs::read(path)?))
}

/// One manifest entry.
#[derive(Debug, Clone)]
pub struct SegInfo {
    pub path: PathBuf,
    pub file: String,
    pub split: String,
    pub ops: u64,
    pub bytes: u64,
}

/// Segments listed in `experiment/traces/<kernel>/MANIFEST.json`, optionally filtered by split
/// ("search" or "holdout").
pub fn manifest_segments(kernel: &str, split: Option<&str>) -> Result<Vec<SegInfo>, String> {
    let dir = traces_dir().join(kernel);
    let text = std::fs::read_to_string(dir.join("MANIFEST.json")).map_err(|e| format!("{kernel}: {e}"))?;
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("{kernel}: {e}"))?;
    let mut out = Vec::new();
    for s in v["segments"].as_array().ok_or("manifest has no segments")? {
        let file = s["file"].as_str().ok_or("segment without file")?.to_string();
        let sp = s["split"].as_str().ok_or("segment without split")?.to_string();
        if split.is_some_and(|w| w != sp) {
            continue;
        }
        out.push(SegInfo {
            path: dir.join(&file),
            file,
            split: sp,
            ops: s["ops"].as_u64().ok_or("segment without ops")?,
            bytes: s["bytes"].as_u64().unwrap_or(0),
        });
    }
    Ok(out)
}

// ---- helpers shared by the kernel specs ----

pub(crate) fn rng(seed: u64) -> ChaCha8Rng {
    ChaCha8Rng::seed_from_u64(seed)
}

/// Uniform-ish value in `0..n` (modulo bias is irrelevant here).
pub(crate) fn below(r: &mut ChaCha8Rng, n: u64) -> u64 {
    r.next_u64() % n
}

pub(crate) fn splitmix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}


/// Encode `value` as a VInt of exactly `width` bytes (1..=9); the value must fit.
pub(crate) fn encode_vint(value: u64, width: usize, out: &mut Vec<u8>) {
    assert!((1..=9).contains(&width));
    if width == 9 {
        out.push(0xFF);
        out.extend_from_slice(&value.to_be_bytes());
        return;
    }
    let extra = width - 1;
    assert!(value < 1u64 << (7 * width));
    let lead = ((0xFFu16 << (8 - extra)) as u8) | (value >> (8 * extra)) as u8;
    out.push(lead);
    for i in (0..extra).rev() {
        out.push((value >> (8 * i)) as u8);
    }
}

/// Smallest and largest value that need a VInt of `width` bytes.
pub(crate) fn vint_range(width: usize) -> (u64, u64) {
    match width {
        1 => (0, 127),
        9 => (1u64 << 56, u64::MAX),
        w => (1u64 << (7 * (w - 1)), (1u64 << (7 * w)) - 1),
    }
}

/// Random value of a given VInt width.
pub(crate) fn random_vint_value(r: &mut ChaCha8Rng, width: usize) -> u64 {
    let (lo, hi) = vint_range(width);
    lo + r.next_u64() % (hi - lo).saturating_add(1).max(1)
}

/// Longest input handed to a kernel together with real trailing context (see the specs'
/// `trace_cases`): the rest of the segment, capped.
pub(crate) const TRAILING_CONTEXT_MAX: usize = 4096;
