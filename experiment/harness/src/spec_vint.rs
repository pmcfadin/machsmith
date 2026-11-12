//! `vint`: `size_t msx_vint(const u8 *ptr, size_t len, u64 *out)`.

use crate::spec::*;
use crate::tramp::{call4, AbiViolation};
use rand_core::RngCore;

type VintFn = unsafe extern "C" fn(*const u8, usize, *mut u64) -> usize;

#[inline(always)]
fn fold(acc: u64, word: u64) -> u64 {
    acc.rotate_left(5) ^ word
}

pub struct VintSpec;

const GARBAGE_D: u64 = 0xBAD0_BAD0_BAD0_BAD0;

impl KernelSpec for VintSpec {
    fn name(&self) -> &'static str {
        "vint"
    }
    fn symbol(&self) -> &'static str {
        "msx_vint"
    }
    fn chain_symbol(&self) -> &'static str {
        "msx_vint_chain"
    }

    unsafe fn run_case(&self, f: RawFn, input: *const u8, len: usize, _aux: u64) -> Result<OutRec, AbiViolation> {
        crate::guard::with_slot(1, |slot| {
        let p = slot.arm();
        let ret = call4(f, input as u64, len as u64, p as u64, GARBAGE_D)?;
        let (out, written) = slot.take();
        Ok(OutRec { ret, out, out_written: written as u64 })
        })
    }

    unsafe fn run_chain(&self, f: RawFn, chunk: &[u8]) -> (u64, u64) {
        let decode: VintFn = core::mem::transmute(f);
        let base = chunk.as_ptr();
        let len = chunk.len();
        let (mut off, mut acc, mut ops) = (0usize, 0u64, 0u64);
        loop {
            let mut v = 0u64;
            let used = decode(base.add(off), len - off, &mut v);
            if used == 0 {
                break;
            }
            acc = fold(acc, v);
            ops += 1;
            off += used;
        }
        (ops, acc)
    }

    unsafe fn chunks<'a>(&self, reference: RawFn, seg: &'a [u8], min_ops: u64) -> Vec<&'a [u8]> {
        let decode: VintFn = core::mem::transmute(reference);
        let mut out = Vec::new();
        let (mut start, mut off, mut n) = (0usize, 0usize, 0u64);
        loop {
            let mut v = 0u64;
            let used = decode(seg.as_ptr().add(off), seg.len() - off, &mut v);
            if used == 0 {
                break;
            }
            off += used;
            n += 1;
            if n >= min_ops {
                out.push(&seg[start..off]);
                start = off;
                n = 0;
            }
        }
        out
    }

    unsafe fn trace_cases(&self, reference: RawFn, seg: &[u8], sink: &mut dyn FnMut(&[u8], u64)) {
        let decode: VintFn = core::mem::transmute(reference);
        let mut off = 0usize;
        loop {
            let mut v = 0u64;
            let used = decode(seg.as_ptr().add(off), seg.len() - off, &mut v);
            if used == 0 {
                break;
            }
            sink(&seg[off..off + used], 0);
            off += used;
        }
    }

    fn edge_cases(&self, sink: &mut dyn FnMut(&[u8], u64)) {
        // Ten 12-byte tails: all 0x00, all 0xFF, eight from a ChaCha stream seeded 0xED6E.
        let mut r = rng(0xED6E);
        let mut tails: Vec<[u8; 12]> = vec![[0x00; 12], [0xFF; 12]];
        for _ in 0..8 {
            let mut t = [0u8; 12];
            r.fill_bytes(&mut t);
            tails.push(t);
        }
        let mut buf;
        for lead in 0..=255u8 {
            for len in 0..=12usize {
                for t in &tails {
                    buf = *t;
                    buf[0] = lead;
                    sink(&buf[..len], 0);
                }
            }
        }
    }

    fn random_cases(&self, seed: u64, n: u64, sink: &mut dyn FnMut(&[u8], u64)) {
        let mut r = rng(seed);
        let mut buf = Vec::with_capacity(80);
        for i in 0..n {
            buf.clear();
            if i % 2 == 0 {
                let len = below(&mut r, 65) as usize;
                buf.resize(len, 0);
                r.fill_bytes(&mut buf);
            } else {
                let width = 1 + below(&mut r, 9) as usize;
                let value = random_vint_value(&mut r, width);
                encode_vint(value, width, &mut buf);
                if r.next_u32() & 1 == 1 {
                    let cut = below(&mut r, buf.len() as u64 + 1) as usize;
                    buf.truncate(cut);
                }
            }
            sink(&buf, 0);
        }
    }
}
