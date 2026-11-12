#[cold]
#[inline(never)]
unsafe fn slow(ptr: *const u8, len: usize, out: *mut u64, first: u8) -> usize {
    let extra = first.leading_ones() as usize; // 1..=8
    if len <= extra {
        return 0;
    }
    let p = ptr.add(1);
    let tail: u64 = if extra == 1 {
        *p as u64
    } else if extra <= 3 {
        let hi = u16::from_be((p as *const u16).read_unaligned()) as u64;
        let lo = u16::from_be((p.add(extra - 2) as *const u16).read_unaligned()) as u64;
        (hi << (8 * (extra - 2))) | lo
    } else {
        let hi = u32::from_be((p as *const u32).read_unaligned()) as u64;
        let lo = u32::from_be((p.add(extra - 4) as *const u32).read_unaligned()) as u64;
        (hi << (8 * (extra - 4))) | lo
    };
    let value = if extra == 8 {
        tail
    } else {
        let mask = (1u64 << (7 - extra)) - 1;
        ((first as u64 & mask) << (extra * 8)) | tail
    };
    *out = value;
    extra + 1
}

#[no_mangle]
pub unsafe extern "C" fn msx_vint(ptr: *const u8, len: usize, out: *mut u64) -> usize {
    if len == 0 {
        return 0;
    }
    let first = *ptr;
    if first < 0x80 {
        *out = first as u64;
        return 1;
    }
    slow(ptr, len, out, first)
}
