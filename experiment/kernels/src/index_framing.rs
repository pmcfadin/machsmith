//! Kernel `index_framing`: CQLite's BIG-format index entry framing parser.

use crate::fold;
use crate::vint::decode_unsigned;
use nom::{bytes::complete::take, number::complete::be_u16, IResult};

// BEGIN VERBATIM cqlite-core/src/parser/vint.rs :: parse_vuint @ 2e5b7a4c
pub fn parse_vuint(input: &[u8]) -> IResult<&[u8], u64> {
    // Issue #1638 (J4): thin nom adapter over the one canonical `decode_unsigned`.
    match decode_unsigned(input) {
        Ok((value, consumed)) => Ok((&input[consumed..], value)),
        Err(_) => Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Eof,
        ))),
    }
}
// END VERBATIM

#[inline(always)] // added on the copy; CQLite's function has no inline attribute, and plain #[inline] left it out of line
// BEGIN VERBATIM cqlite-core/src/storage/sstable/index_reader/parse.rs :: parse_big_index_entry_framing @ 2e5b7a4c
pub fn parse_big_index_entry_framing(input: &[u8]) -> IResult<&[u8], (usize, u64)> {
    let original_len = input.len();
    let (input, key_len) = be_u16(input)?;
    let (input, _key_bytes) = take(key_len)(input)?;
    let (input, _data_offset) = parse_vuint(input)?;
    let (remaining, promoted_len) = parse_vuint(input)?;
    let framing_len = original_len - remaining.len();
    Ok((remaining, (framing_len, promoted_len)))
}
// END VERBATIM

#[repr(C)]
pub struct FramingOut {
    pub framing_len: u64,
    pub promoted_len: u64,
}

/// Returns 0 on success and writes `*out`. On a nom Error/Failure returns
/// `1 + (ErrorKind as u32)`; on Incomplete returns `u32::MAX`. `*out` is not
/// written on error.
///
/// # Safety
/// `ptr` valid for `len` bytes (or `len == 0`); `out` valid for a `FramingOut` write.
#[no_mangle]
pub unsafe extern "C" fn msx_index_framing(ptr: *const u8, len: usize, out: *mut FramingOut) -> u32 {
    let input = core::slice::from_raw_parts(ptr, len);
    match parse_big_index_entry_framing(input) {
        Ok((_, (framing_len, promoted_len))) => {
            *out = FramingOut {
                framing_len: framing_len as u64,
                promoted_len,
            };
            0
        }
        Err(nom::Err::Error(e)) | Err(nom::Err::Failure(e)) => 1 + (e.code as u32),
        Err(nom::Err::Incomplete(_)) => u32::MAX,
    }
}

/// Walks consecutive index entries in `seg`: parse; on error stop; fold
/// `framing_len` then `promoted_len`; advance by `framing_len + promoted_len`, and
/// stop if that passes the end of the segment. Returns the number of entries
/// parsed (the last one counts even if it passes the end).
///
/// # Safety
/// `seg` valid for `seg_len` bytes; `checksum` valid for a `u64` write.
#[no_mangle]
pub unsafe extern "C" fn msx_index_framing_chain(seg: *const u8, seg_len: usize, checksum: *mut u64) -> u64 {
    let seg = core::slice::from_raw_parts(seg, seg_len);
    let mut off = 0usize;
    let mut acc = 0u64;
    let mut ops = 0u64;
    while let Ok((_, (framing_len, promoted_len))) = parse_big_index_entry_framing(&seg[off..]) {
        acc = fold(acc, framing_len as u64);
        acc = fold(acc, promoted_len);
        ops += 1;
        let step = (framing_len as u64).saturating_add(promoted_len);
        if step > (seg_len - off) as u64 {
            break;
        }
        off += step as usize;
    }
    *checksum = acc;
    ops
}
