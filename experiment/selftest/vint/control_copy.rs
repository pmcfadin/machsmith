// Control candidate: CQLite's `decode_unsigned` verbatim, behind the experiment's C ABI.
// Written against `core` only; the wrapper lib.rs supplies the crate attributes.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VIntError {
    Truncated,
}

#[inline]
pub fn decode_unsigned(input: &[u8]) -> Result<(u64, usize), VIntError> {
    let first = *input.first().ok_or(VIntError::Truncated)?;
    let extra = first.leading_ones() as usize;
    let total = extra + 1;
    if input.len() < total {
        return Err(VIntError::Truncated);
    }
    if extra == 0 {
        return Ok((first as u64, 1));
    }
    let (_, rest) = input.split_at(1);
    let mut be = [0u8; 8];
    be[8 - extra..].copy_from_slice(&rest[..extra]);
    let tail = u64::from_be_bytes(be);
    let value = if extra == 8 {
        tail
    } else {
        let data_bits_first = 7 - extra;
        let mask = (1u64 << data_bits_first) - 1;
        ((first as u64 & mask) << (extra * 8)) | tail
    };
    Ok((value, total))
}

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
