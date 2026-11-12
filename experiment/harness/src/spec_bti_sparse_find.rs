//! `bti_sparse_find`: `u32 msx_bti_sparse_find(const Transition *t, size_t n, u32 byte, u64 *out)`.
//!
//! The input of a case is `n * 16` bytes of `Transition` (byte at offset 0, seven padding
//! bytes, child `u64` at offset 8); `aux` carries the `byte` argument (upper 24 bits garbage).

use crate::guard::OutSlot;
use crate::spec::*;
use crate::tramp::{call4, AbiViolation};
use rand_chacha::ChaCha8Rng;
use rand_core::RngCore;

type FindFn = unsafe extern "C" fn(*const u8, usize, u32, *mut u64) -> u32;

#[inline(always)]
fn fold(acc: u64, word: u64) -> u64 {
    acc.rotate_left(5) ^ word
}

pub struct BtiSparseFindSpec;

const EDGE_NS: [usize; 32] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 31, 32, 33, 63, 64, 65, 127, 128, 129,
    255, 256,
];

fn rd_header(p: &[u8]) -> (usize, u32) {
    (
        u32::from_le_bytes(p[0..4].try_into().unwrap()) as usize,
        u32::from_le_bytes(p[4..8].try_into().unwrap()),
    )
}

/// `n` sorted distinct random bytes; for `n == 256` that is every byte.
fn sorted_bytes(r: &mut ChaCha8Rng, n: usize) -> Vec<u8> {
    let mut all: Vec<u8> = (0..=255u8).collect();
    for i in (1..all.len()).rev() {
        all.swap(i, below(r, i as u64 + 1) as usize);
    }
    all.truncate(n);
    all.sort_unstable();
    all
}

/// Serialize transitions: byte, seven random padding bytes, random child.
fn build_input(r: &mut ChaCha8Rng, bytes: &[u8], out: &mut Vec<u8>) {
    out.clear();
    for &b in bytes {
        let mut rec = [0u8; 16];
        r.fill_bytes(&mut rec);
        rec[0] = b;
        out.extend_from_slice(&rec);
    }
}

impl KernelSpec for BtiSparseFindSpec {
    fn name(&self) -> &'static str {
        "bti_sparse_find"
    }
    fn symbol(&self) -> &'static str {
        "msx_bti_sparse_find"
    }
    fn chain_symbol(&self) -> &'static str {
        "msx_bti_sparse_find_chain"
    }

    unsafe fn run_case(&self, f: RawFn, input: *const u8, len: usize, aux: u64) -> Result<OutRec, AbiViolation> {
        let mut slot = OutSlot::new(1);
        let p = slot.arm();
        // `byte` is a 32-bit argument: the upper half of its register is undefined, so fill it with garbage.
        let c = (aux & 0xFFFF_FFFF) | (splitmix(aux) << 32);
        let d = p as u64;
        let ret = call4(f, input as u64, (len / 16) as u64, c, d)? & 0xFFFF_FFFF; // u32 return
        let (out, written) = slot.take();
        Ok(OutRec { ret, out, out_written: written as u64 })
    }

    unsafe fn run_chain(&self, f: RawFn, chunk: &[u8]) -> (u64, u64) {
        debug_assert_eq!(chunk.as_ptr() as usize % 8, 0, "bti chain needs an 8-byte-aligned base");
        let find: FindFn = core::mem::transmute(f);
        let len = chunk.len();
        let base = chunk.as_ptr();
        let (mut off, mut acc, mut ops) = (0usize, 0u64, 0u64);
        while len - off >= 8 {
            let p = base.add(off);
            let (n, byte) = rd_header(core::slice::from_raw_parts(p, 8));
            if len - off - 8 < n * 16 {
                break;
            }
            let mut child = 0u64;
            let found = find(p.add(8), n, byte, &mut child);
            acc = fold(acc, found as u64);
            if found != 0 {
                acc = fold(acc, child);
            }
            ops += 1;
            off += 8 + n * 16;
        }
        (ops, acc)
    }

    unsafe fn chunks<'a>(&self, _reference: RawFn, seg: &'a [u8], min_ops: u64) -> Vec<&'a [u8]> {
        // Boundaries come from the record framing alone; the reference is not consulted.
        let mut out = Vec::new();
        let (mut start, mut off, mut n) = (0usize, 0usize, 0u64);
        while seg.len() - off >= 8 {
            let (cnt, _) = rd_header(&seg[off..]);
            if seg.len() - off - 8 < cnt * 16 {
                break;
            }
            off += 8 + cnt * 16;
            n += 1;
            if n >= min_ops {
                out.push(&seg[start..off]);
                start = off;
                n = 0;
            }
        }
        out
    }

    unsafe fn trace_cases(&self, _reference: RawFn, seg: &[u8], sink: &mut dyn FnMut(&[u8], u64)) {
        let mut off = 0usize;
        while seg.len() - off >= 8 {
            let (cnt, byte) = rd_header(&seg[off..]);
            if seg.len() - off - 8 < cnt * 16 {
                break;
            }
            sink(&seg[off + 8..off + 8 + cnt * 16], byte as u64);
            off += 8 + cnt * 16;
        }
    }

    fn edge_cases(&self, sink: &mut dyn FnMut(&[u8], u64)) {
        let mut r = rng(0xED70);
        let mut input = Vec::new();
        for &n in &EDGE_NS {
            let bytes = sorted_bytes(&mut r, n);
            build_input(&mut r, &bytes, &mut input);
            for q in 0..=255u64 {
                let garbage = r.next_u32() as u64 & 0x00FF_FFFF;
                sink(&input, q | (garbage << 8));
            }
        }
    }

    fn random_cases(&self, seed: u64, n: u64, sink: &mut dyn FnMut(&[u8], u64)) {
        let mut r = rng(seed);
        let mut input = Vec::new();
        for i in 0..n {
            // Inputs must be sorted with distinct keys (the kernel's contract), so the "uniform"
            // half uses small tables with arbitrary query bytes and the structural half larger
            // tables whose queries hit a present key about half the time.
            let uniform = i % 2 == 0;
            let cnt = if uniform { below(&mut r, 5) } else { below(&mut r, 41) } as usize;
            let bytes = sorted_bytes(&mut r, cnt);
            build_input(&mut r, &bytes, &mut input);
            let q = if !uniform && !bytes.is_empty() && r.next_u32() & 1 == 1 {
                bytes[below(&mut r, bytes.len() as u64) as usize] as u64
            } else {
                r.next_u32() as u64 & 0xFF
            };
            let garbage = r.next_u32() as u64 & 0x00FF_FFFF;
            sink(&input, q | (garbage << 8));
        }
    }
}
