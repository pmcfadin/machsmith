use msx_kernels::bti_sparse_find::{msx_bti_sparse_find, msx_bti_sparse_find_chain, Transition};
use msx_kernels::index_framing::{
    msx_index_framing, msx_index_framing_chain, parse_big_index_entry_framing, FramingOut,
};
use msx_kernels::vint::{msx_vint, msx_vint_chain};
use nom::error::ErrorKind;
use rand_chacha::ChaCha8Rng;
use rand_core::{RngCore, SeedableRng};

const SENTINEL: u64 = 0xDEAD_BEEF_DEAD_BEEF;
const EOF_STATUS: u32 = 1 + ErrorKind::Eof as u32;

fn vint(input: &[u8]) -> (usize, u64) {
    let mut out = SENTINEL;
    let r = unsafe { msx_vint(input.as_ptr(), input.len(), &mut out) };
    (r, out)
}

#[test]
fn vint_table() {
    let cases: &[(&[u8], usize, u64)] = &[
        (&[0x00], 1, 0),
        (&[0x7F], 1, 127),
        (&[0x80, 0x80], 2, 128),
        (&[0xBF, 0xFF], 2, 0x3FFF),
        (&[0xFE, 1, 2, 3, 4, 5, 6, 7], 8, 0x0001_0203_0405_0607),
        (&[0xFF, 1, 2, 3, 4, 5, 6, 7, 8], 9, 0x0102_0304_0506_0708),
        (&[0x00, 0xAA], 1, 0),
        (&[], 0, SENTINEL),
        (&[0x80], 0, SENTINEL),
        (&[0xFF, 0, 0, 0, 0, 0, 0, 0], 0, SENTINEL),
    ];
    for (input, ret, out) in cases {
        assert_eq!(vint(input), (*ret, *out), "input {input:02x?}");
    }
}

#[test]
fn vint_chain_checksum() {
    let seg = [0x00u8, 0x80, 0x80, 0x7F];
    let mut sum = 0u64;
    let ops = unsafe { msx_vint_chain(seg.as_ptr(), seg.len(), &mut sum) };
    assert_eq!(ops, 3);
    // values 0, 128, 127: acc = 0; rotl(0,5)^128 = 128; rotl(128,5)^127 = 4096^127.
    assert_eq!(sum, (128u64 << 5) ^ 127);
    assert_eq!(sum, 4223);
}

#[test]
fn vint_chain_stops_on_truncation() {
    let seg = [0x01u8, 0x02, 0x80];
    let mut sum = 0u64;
    let ops = unsafe { msx_vint_chain(seg.as_ptr(), seg.len(), &mut sum) };
    assert_eq!(ops, 2);
    assert_eq!(sum, (1u64 << 5) ^ 2);
}

fn framing(input: &[u8]) -> (u32, u64, u64) {
    let mut out = FramingOut { framing_len: SENTINEL, promoted_len: SENTINEL };
    let r = unsafe { msx_index_framing(input.as_ptr(), input.len(), &mut out) };
    (r, out.framing_len, out.promoted_len)
}

#[test]
fn index_framing_table() {
    let a = [0x00u8, 0x02, b'a', b'b', 0x05, 0x00];
    assert_eq!(framing(&a), (0, 6, 0));
    let mut b = a.to_vec();
    b.push(0x99);
    assert_eq!(framing(&b), (0, 6, 0));
    assert_eq!(framing(&[0x00, 0x00, 0x05, 0x81, 0x00]), (0, 5, 256));
    for l in 0..=5 {
        assert_eq!(framing(&a[..l]), (EOF_STATUS, SENTINEL, SENTINEL), "prefix {l}");
    }
}

#[test]
fn index_framing_chain_checksum() {
    // entry 1: framing 6, promoted 3 (3 payload bytes); entry 2: framing 5, promoted 0.
    let seg = [0x00u8, 0x02, b'a', b'b', 0x05, 0x03, 9, 9, 9, 0x00, 0x00, 0x05, 0x00];
    let mut sum = 0u64;
    let ops = unsafe { msx_index_framing_chain(seg.as_ptr(), seg.len(), &mut sum) };
    assert_eq!(ops, 2);
    let mut acc = 0u64;
    for w in [6u64, 3, 4, 0] {
        acc = acc.rotate_left(5) ^ w;
    }
    // second entry is 0x00 0x00 0x05 0x00 -> framing 4
    assert_eq!(sum, acc);
    // payload that passes the end stops the chain after counting the entry.
    let seg = [0x00u8, 0x00, 0x05, 0x09, 1];
    let ops = unsafe { msx_index_framing_chain(seg.as_ptr(), seg.len(), &mut sum) };
    assert_eq!(ops, 1);
    assert_eq!(sum, (4u64 << 5) ^ 9);
}

fn translate(r: nom::IResult<&[u8], (usize, u64)>) -> (u32, u64, u64) {
    match r {
        Ok((_, (f, p))) => (0, f as u64, p),
        Err(nom::Err::Error(e)) | Err(nom::Err::Failure(e)) => (1 + e.code as u32, SENTINEL, SENTINEL),
        Err(nom::Err::Incomplete(_)) => (u32::MAX, SENTINEL, SENTINEL),
    }
}

#[test]
fn index_framing_matches_cqlite_function_random() {
    let mut rng = ChaCha8Rng::seed_from_u64(0x1F);
    for i in 0..10_000 {
        let len = (rng.next_u32() % 41) as usize;
        let mut v = vec![0u8; len];
        rng.fill_bytes(&mut v);
        // Bias a third of the cases towards well-formed prefixes so success is exercised.
        if i % 3 == 0 && len >= 2 {
            v[0] = 0;
            v[1] = (v[1] % 6) as u8;
        }
        assert_eq!(framing(&v), translate(parse_big_index_entry_framing(&v)), "input {v:02x?}");
    }
}

/// Transition list with padding bytes filled with 0xA5, in 8-aligned storage.
fn pack(ts: &[(u8, u64)]) -> Vec<u64> {
    let mut words = vec![0xA5A5_A5A5_A5A5_A5A5u64; ts.len() * 2];
    for (i, (b, c)) in ts.iter().enumerate() {
        // byte lives in the low byte of the first word; bytes 1..8 stay 0xA5 padding.
        words[2 * i] = (words[2 * i] & !0xFF) | *b as u64;
        words[2 * i + 1] = *c;
    }
    words
}

fn find(words: &[u64], n: usize, byte: u32) -> (u32, u64) {
    let mut out = SENTINEL;
    let r = unsafe { msx_bti_sparse_find(words.as_ptr() as *const Transition, n, byte, &mut out) };
    (r, out)
}

#[test]
fn bti_layout() {
    assert_eq!(std::mem::size_of::<Transition>(), 16);
    assert_eq!(std::mem::align_of::<Transition>(), 8);
}

#[test]
fn bti_sparse_table() {
    let w = pack(&[(0x10, 100), (0x20, 200), (0x30, 300)]);
    assert_eq!(find(&w, 3, 0x10), (1, 100));
    assert_eq!(find(&w, 3, 0x20), (1, 200));
    assert_eq!(find(&w, 3, 0x30), (1, 300));
    for b in [0x0F, 0x11, 0x2F, 0x31, 0xFF] {
        assert_eq!(find(&w, 3, b), (0, SENTINEL), "byte {b:#x}");
    }
    assert_eq!(find(&w, 3, 0xABCD_EF20), (1, 200));
    assert_eq!(find(&w, 0, 0x10), (0, SENTINEL));
    assert_eq!(find(&w, 1, 0x10), (1, 100));
    assert_eq!(find(&w, 1, 0x20), (0, SENTINEL));
}

#[test]
fn bti_matches_binary_search_random() {
    let mut rng = ChaCha8Rng::seed_from_u64(0xB71);
    for _ in 0..10_000 {
        let n = (rng.next_u32() % 257) as usize;
        let mut bytes: Vec<u8> = (0..=255u8).collect();
        for i in (1..256).rev() {
            bytes.swap(i, (rng.next_u32() as usize) % (i + 1));
        }
        bytes.truncate(n);
        bytes.sort_unstable();
        let ts: Vec<(u8, u64)> = bytes.iter().map(|&b| (b, rng.next_u64())).collect();
        let w = pack(&ts);
        for _ in 0..8 {
            let q = rng.next_u32();
            let expect = ts.binary_search_by_key(&(q as u8), |t| t.0).ok().map(|i| ts[i].1);
            let got = find(&w, n, q);
            match expect {
                Some(c) => assert_eq!(got, (1, c)),
                None => assert_eq!(got, (0, SENTINEL)),
            }
        }
    }
}

#[test]
fn bti_chain_checksum() {
    // record 1: n=2 byte=0x20 -> found child 7; record 2: n=1 byte=0x05 -> not found;
    // trailing 4 bytes cannot hold a header; a third record with a short payload also stops.
    let mut words: Vec<u64> = Vec::new();
    let mut push_rec = |n: u32, byte: u32, ts: &[(u8, u64)]| {
        words.push(n as u64 | (byte as u64) << 32);
        words.extend(pack(ts));
    };
    push_rec(2, 0x20, &[(0x10, 5), (0x20, 7)]);
    push_rec(1, 0x05, &[(0x10, 9)]);
    words.push(3u64); // header: n=3, byte=0, but no payload follows
    let seg_len = words.len() * 8;
    let mut sum = 0u64;
    let ops = unsafe { msx_bti_sparse_find_chain(words.as_ptr() as *const u8, seg_len, &mut sum) };
    assert_eq!(ops, 2);
    let mut acc = 0u64;
    for w in [1u64, 7, 0] {
        acc = acc.rotate_left(5) ^ w;
    }
    assert_eq!(sum, acc);
    // too short for even a header
    let ops = unsafe { msx_bti_sparse_find_chain(words.as_ptr() as *const u8, 7, &mut sum) };
    assert_eq!((ops, sum), (0, 0));
}
