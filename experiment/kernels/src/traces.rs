//! Trace segments: walking, the search/holdout split rule, and the manifest types.
//!
//! Used by the `msx-trace-manifest` tool and by `tests/traces.rs`.

use crate::bti_sparse_find::{find_sparse, Transition};
use crate::index_framing::parse_big_index_entry_framing;
use crate::vint::decode_unsigned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// A segment goes to `holdout` when the first byte of the SHA-256 of its content is below this.
pub const DEFAULT_SPLIT_THRESHOLD: u8 = 77;
pub const SAMPLE_MIN_OPS: u64 = 50_000;
pub const SMALL_TRACE_MIN_CHUNKS: u64 = 20;

pub const KERNELS: [&str; 3] = ["vint", "index_framing", "bti_sparse_find"];

pub fn sha256(content: &[u8]) -> [u8; 32] {
    Sha256::digest(content).into()
}

pub fn hex(d: &[u8]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Split {
    Search,
    Holdout,
}

impl Split {
    pub fn dir(self) -> &'static str {
        match self {
            Split::Search => "search",
            Split::Holdout => "holdout",
        }
    }
}

pub fn split_for_with(content: &[u8], threshold: u8) -> Split {
    if sha256(content)[0] < threshold {
        Split::Holdout
    } else {
        Split::Search
    }
}

/// The split rule with the default threshold (77, about 30% holdout).
pub fn split_for(content: &[u8]) -> Split {
    split_for_with(content, DEFAULT_SPLIT_THRESHOLD)
}

/// Per-segment statistics gathered while walking.
#[derive(Default, Clone, Debug)]
pub struct Stats {
    /// vint: operations by encoded length 1..=9 (index 0 unused).
    pub vint_len: [u64; 10],
    /// index_framing: key length and promoted length of each entry.
    pub key_lens: Vec<u64>,
    pub promoted_lens: Vec<u64>,
    /// bti_sparse_find: `n` of each call, and whether the byte was found.
    pub bti_n: Vec<u64>,
    pub bti_hits: u64,
}

/// Walk `bytes` as a chain for `kernel`; succeed only if the walk consumes the whole segment.
/// Returns the number of operations.
pub fn walk(kernel: &str, bytes: &[u8]) -> Result<u64, String> {
    walk_stats(kernel, bytes, None)
}

pub fn walk_stats(kernel: &str, bytes: &[u8], mut st: Option<&mut Stats>) -> Result<u64, String> {
    let len = bytes.len();
    let mut off = 0usize;
    let mut ops = 0u64;
    match kernel {
        "vint" => {
            while off < len {
                match decode_unsigned(&bytes[off..]) {
                    Ok((_, used)) => {
                        if let Some(s) = st.as_deref_mut() {
                            s.vint_len[used] += 1;
                        }
                        off += used;
                        ops += 1;
                    }
                    Err(e) => {
                        return Err(format!(
                            "walk ended {} bytes before end of segment (error {e:?} after {ops} ops)",
                            len - off
                        ))
                    }
                }
            }
        }
        "index_framing" => {
            while off < len {
                match parse_big_index_entry_framing(&bytes[off..]) {
                    Ok((_, (framing_len, promoted_len))) => {
                        let key_len = u16::from_be_bytes([bytes[off], bytes[off + 1]]) as u64;
                        let step = (framing_len as u64).saturating_add(promoted_len);
                        if step > (len - off) as u64 {
                            return Err(format!(
                                "walk error: entry at offset {off} passes the end by {} bytes",
                                step - (len - off) as u64
                            ));
                        }
                        if let Some(s) = st.as_deref_mut() {
                            s.key_lens.push(key_len);
                            s.promoted_lens.push(promoted_len);
                        }
                        off += step as usize;
                        ops += 1;
                    }
                    Err(e) => {
                        let kind = match e {
                            nom::Err::Error(x) | nom::Err::Failure(x) => format!("{:?}", x.code),
                            nom::Err::Incomplete(_) => "Incomplete".to_string(),
                        };
                        return Err(format!(
                            "walk ended {} bytes before end of file (parse error {kind} after {ops} entries)",
                            len - off
                        ))
                    }
                }
            }
        }
        "bti_sparse_find" => {
            while off < len {
                if len - off < 8 {
                    return Err(format!("walk ended {} bytes before end of segment (short header)", len - off));
                }
                let n = u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap()) as usize;
                let byte = u32::from_le_bytes(bytes[off + 4..off + 8].try_into().unwrap());
                let payload = n.checked_mul(16).ok_or("record size overflow")?;
                if len - off - 8 < payload {
                    return Err(format!(
                        "walk ended {} bytes before end of segment (record needs {} payload bytes)",
                        len - off,
                        payload
                    ));
                }
                let ts: Vec<Transition> = (0..n)
                    .map(|i| {
                        let e = off + 8 + i * 16;
                        Transition {
                            byte: bytes[e],
                            child: u64::from_le_bytes(bytes[e + 8..e + 16].try_into().unwrap()),
                        }
                    })
                    .collect();
                let hit = find_sparse(&ts, byte as u8).is_some();
                if let Some(s) = st.as_deref_mut() {
                    s.bti_n.push(n as u64);
                    s.bti_hits += hit as u64;
                }
                off += 8 + payload;
                ops += 1;
            }
        }
        other => return Err(format!("unknown kernel {other}")),
    }
    Ok(ops)
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SegmentEntry {
    pub file: String,
    pub origin: String,
    pub split: Split,
    pub sha256: String,
    pub bytes: u64,
    pub ops: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Excluded {
    pub origin: String,
    pub reason: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct Totals {
    pub search_ops: u64,
    pub holdout_ops: u64,
    pub search_bytes: u64,
    pub holdout_bytes: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct SmallTrace {
    pub search: bool,
    pub holdout: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Manifest {
    pub kernel: String,
    pub cqlite_commit: String,
    pub capture_commands: Vec<String>,
    pub format: String,
    /// Present only when the default threshold (77) was changed by the size rules.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub split_threshold: Option<u8>,
    pub segments: Vec<SegmentEntry>,
    pub excluded: Vec<Excluded>,
    pub duplicates_dropped: u64,
    pub totals: Totals,
    pub small_trace: SmallTrace,
    pub histograms: serde_json::Value,
}

/// True when a split of `ops` operations yields fewer than `SMALL_TRACE_MIN_CHUNKS`
/// chunks of `SAMPLE_MIN_OPS`.
pub fn is_small_trace(ops: u64) -> bool {
    ops / SAMPLE_MIN_OPS < SMALL_TRACE_MIN_CHUNKS
}
