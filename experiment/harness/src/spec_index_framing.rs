//! `index_framing`: `u32 msx_index_framing(const u8 *ptr, size_t len, FramingOut *out)`.

use crate::spec::*;
use crate::tramp::{call4, AbiViolation};
use rand_core::RngCore;

#[repr(C)]
struct FramingOut {
    framing_len: u64,
    promoted_len: u64,
}

type FramingFn = unsafe extern "C" fn(*const u8, usize, *mut FramingOut) -> u32;

#[inline(always)]
fn fold(acc: u64, word: u64) -> u64 {
    acc.rotate_left(5) ^ word
}

pub struct IndexFramingSpec;

const GARBAGE_D: u64 = 0xBAD1_BAD1_BAD1_BAD1;

const KEY_LENS: [usize; 20] = [0, 1, 2, 3, 7, 8, 9, 15, 16, 17, 31, 32, 33, 63, 64, 65, 255, 256, 1000, 65535];

/// `[u16 BE key_len][key][vint offset][vint promoted]`.
fn build_entry(r: &mut impl RngCore, key_len: usize, off: (u64, usize), prom: (u64, usize), out: &mut Vec<u8>) {
    out.clear();
    out.extend_from_slice(&(key_len as u16).to_be_bytes());
    let start = out.len();
    out.resize(start + key_len, 0);
    r.fill_bytes(&mut out[start..]);
    encode_vint(off.0, off.1, out);
    encode_vint(prom.0, prom.1, out);
}

impl KernelSpec for IndexFramingSpec {
    fn name(&self) -> &'static str {
        "index_framing"
    }
    fn symbol(&self) -> &'static str {
        "msx_index_framing"
    }
    fn chain_symbol(&self) -> &'static str {
        "msx_index_framing_chain"
    }

    unsafe fn run_case(&self, f: RawFn, input: *const u8, len: usize, _aux: u64) -> Result<OutRec, AbiViolation> {
        crate::guard::with_slot(2, |slot| {
        let p = slot.arm();
        let ret = call4(f, input as u64, len as u64, p as u64, GARBAGE_D)? & 0xFFFF_FFFF; // u32 return
        let (out, written) = slot.take();
        Ok(OutRec { ret, out, out_written: written as u64 })
        })
    }

    unsafe fn run_chain(&self, f: RawFn, chunk: &[u8]) -> (u64, u64) {
        let parse: FramingFn = core::mem::transmute(f);
        let base = chunk.as_ptr();
        let len = chunk.len();
        let (mut off, mut acc, mut ops) = (0usize, 0u64, 0u64);
        let mut o = FramingOut { framing_len: 0, promoted_len: 0 };
        while parse(base.add(off), len - off, &mut o) == 0 {
            acc = fold(acc, o.framing_len);
            acc = fold(acc, o.promoted_len);
            ops += 1;
            let step = o.framing_len.saturating_add(o.promoted_len);
            if step > (len - off) as u64 {
                break;
            }
            off += step as usize;
        }
        (ops, acc)
    }

    unsafe fn chunks<'a>(&self, reference: RawFn, seg: &'a [u8], min_ops: u64) -> Vec<&'a [u8]> {
        let parse: FramingFn = core::mem::transmute(reference);
        let mut out = Vec::new();
        let (mut start, mut off, mut n) = (0usize, 0usize, 0u64);
        let mut o = FramingOut { framing_len: 0, promoted_len: 0 };
        while parse(seg.as_ptr().add(off), seg.len() - off, &mut o) == 0 {
            n += 1;
            let step = o.framing_len.saturating_add(o.promoted_len);
            let past_end = step > (seg.len() - off) as u64;
            off = if past_end { seg.len() } else { off + step as usize };
            if n >= min_ops {
                out.push(&seg[start..off]);
                start = off;
                n = 0;
            }
            if past_end {
                break;
            }
        }
        out
    }

    unsafe fn trace_cases(&self, reference: RawFn, seg: &[u8], sink: &mut dyn FnMut(&[u8], u64)) {
        let parse: FramingFn = core::mem::transmute(reference);
        let mut off = 0usize;
        let mut o = FramingOut { framing_len: 0, promoted_len: 0 };
        while parse(seg.as_ptr().add(off), seg.len() - off, &mut o) == 0 {
            // The call reads only the framing bytes; hand exactly those to the kernel under test.
            sink(&seg[off..off + o.framing_len as usize], 0);
            let step = o.framing_len.saturating_add(o.promoted_len);
            if step > (seg.len() - off) as u64 {
                break;
            }
            off += step as usize;
        }
    }

    fn edge_cases(&self, sink: &mut dyn FnMut(&[u8], u64)) {
        let mut r = rng(0xED6F);
        let mut entry = Vec::new();
        let mut cuts: Vec<usize> = Vec::new();
        for &kl in &KEY_LENS {
            for ow in 1..=9usize {
                let (olo, ohi) = vint_range(ow);
                for ov in [olo, ohi] {
                    for pw in 1..=9usize {
                        let (plo, phi) = vint_range(pw);
                        for pv in [plo, phi] {
                            build_entry(&mut r, kl, (ov, ow), (pv, pw), &mut entry);
                            let total = entry.len();
                            cuts.clear();
                            if total <= 300 {
                                cuts.extend(0..=total);
                            } else {
                                let key_end = 2 + kl;
                                cuts.extend(0..=4);
                                cuts.extend(key_end.saturating_sub(2)..=key_end + 2);
                                cuts.extend(total.saturating_sub(19)..=total);
                                cuts.retain(|&c| c <= total);
                                cuts.sort_unstable();
                                cuts.dedup();
                            }
                            for &c in &cuts {
                                sink(&entry[..c], 0);
                            }
                        }
                    }
                }
            }
        }
    }

    fn random_cases(&self, seed: u64, n: u64, sink: &mut dyn FnMut(&[u8], u64)) {
        let mut r = rng(seed);
        let mut buf = Vec::with_capacity(512);
        for i in 0..n {
            buf.clear();
            if i % 2 == 0 {
                let len = below(&mut r, 65) as usize;
                buf.resize(len, 0);
                r.fill_bytes(&mut buf);
            } else {
                let kl = if below(&mut r, 8) == 0 { below(&mut r, 400) } else { below(&mut r, 48) } as usize;
                let ow = 1 + below(&mut r, 9) as usize;
                let pw = 1 + below(&mut r, 9) as usize;
                let ov = random_vint_value(&mut r, ow);
                let pv = random_vint_value(&mut r, pw);
                build_entry(&mut r, kl, (ov, ow), (pv, pw), &mut buf);
                if r.next_u32() & 1 == 1 {
                    let cut = below(&mut r, buf.len() as u64 + 1) as usize;
                    buf.truncate(cut);
                }
            }
            sink(&buf, 0);
        }
    }
}
