//! Kernel `vint`: CQLite's canonical unsigned VInt decoder.

use crate::fold;

// BEGIN VERBATIM cqlite-core/src/parser/vint.rs :: VIntError @ 2e5b7a4c
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VIntError {
    /// Input ended before the full VInt could be read (covers empty input).
    Truncated,
}
// END VERBATIM

// BEGIN VERBATIM cqlite-core/src/parser/vint.rs :: decode_unsigned @ 2e5b7a4c
#[inline]
pub fn decode_unsigned(input: &[u8]) -> Result<(u64, usize), VIntError> {
    let first = *input.first().ok_or(VIntError::Truncated)?;
    // A u8 has at most 8 leading ones, so `extra ∈ 0..=8` and `total ∈ 1..=9`
    // (== MAX_VINT_SIZE). No width can exceed the 9-byte VInt maximum.
    let extra = first.leading_ones() as usize;
    let total = extra + 1;
    if input.len() < total {
        return Err(VIntError::Truncated);
    }
    if extra == 0 {
        // Single byte: 0xxxxxxx
        return Ok((first as u64, 1));
    }
    // Load the `extra` continuation bytes as the low bits of a u64 via ONE
    // big-endian read. `rest[..extra]` is in-bounds: `input.len() >= total`.
    let (_, rest) = input.split_at(1);
    let mut be = [0u8; 8];
    be[8 - extra..].copy_from_slice(&rest[..extra]);
    let tail = u64::from_be_bytes(be);
    let value = if extra == 8 {
        // 0xFF lead: no data bits in the first byte; all 8 continuation bytes.
        tail
    } else {
        // Data bits carried in the first byte occupy the low `7 - extra` bits
        // (extra ∈ 1..=7 ⇒ data_bits_first ∈ 0..=6, so `1 << data_bits_first`
        // never overflows; extra == 7 ⇒ mask == 0, i.e. the 0xFE no-data case).
        let data_bits_first = 7 - extra;
        let mask = (1u64 << data_bits_first) - 1;
        // extra*8 ≤ 56 and (first & mask) ≤ 6 bits ⇒ ≤ 62 significant bits: no
        // shift overflow.
        ((first as u64 & mask) << (extra * 8)) | tail
    };
    Ok((value, total))
}
// END VERBATIM

/// Returns bytes consumed (1..=9) and writes the value to `*out`; returns 0 (and
/// writes nothing) if the input is empty or shorter than the lead byte declares.
///
/// # Safety
/// `ptr` must be valid for `len` bytes (or `len == 0`); `out` must be valid for a
/// `u64` write.
#[no_mangle]
pub unsafe extern "C" fn msx_vint(ptr: *const u8, len: usize, out: *mut u64) -> usize {
    let input = core::slice::from_raw_parts(ptr, len);
    match decode_unsigned(input) {
        Ok((value, consumed)) => {
            *out = value;
            consumed
        }
        Err(VIntError::Truncated) => 0,
    }
}

/// Decodes consecutive VInts from `seg` until one fails; folds each value into
/// the checksum. Writes the final checksum to `*checksum`; returns the number of
/// values decoded.
///
/// # Safety
/// `seg` valid for `seg_len` bytes; `checksum` valid for a `u64` write.
#[no_mangle]
pub unsafe extern "C" fn msx_vint_chain(seg: *const u8, seg_len: usize, checksum: *mut u64) -> u64 {
    let seg = core::slice::from_raw_parts(seg, seg_len);
    let mut off = 0usize;
    let mut acc = 0u64;
    let mut ops = 0u64;
    while let Ok((value, consumed)) = decode_unsigned(&seg[off..]) {
        acc = fold(acc, value);
        ops += 1;
        off += consumed;
    }
    *checksum = acc;
    ops
}
