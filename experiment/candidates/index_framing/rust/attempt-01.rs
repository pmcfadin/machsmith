#[repr(C)]
pub struct FramingOut {
    pub framing_len: u64,
    pub promoted_len: u64,
}

const EOF_STATUS: u32 = 24;

#[cold]
#[inline(never)]
unsafe fn wide_tail(p: *const u8, len: usize, pos: usize, out: *mut FramingOut) -> u32 {
    // p[pos] >= 0x80, pos < len
    let first = *p.add(pos);
    let extra = first.leading_ones() as usize;
    let end = pos + extra + 1;
    if end > len {
        return EOF_STATUS;
    }
    let mut tail: u64 = 0;
    let mut i = 1;
    while i <= extra {
        tail = (tail << 8) | (*p.add(pos + i) as u64);
        i += 1;
    }
    let value = if extra == 8 {
        tail
    } else {
        let m = (first as u64) & (0x7Fu64 >> extra);
        (m << (extra * 8)) | tail
    };
    (*out).framing_len = end as u64;
    (*out).promoted_len = value;
    0
}

#[no_mangle]
pub unsafe extern "C" fn msx_index_framing(ptr: *const u8, len: usize, out: *mut FramingOut) -> u32 {
    if len < 2 {
        return EOF_STATUS;
    }
    let kl = u16::from_be_bytes([*ptr, *ptr.add(1)]) as usize;
    let pos = 2 + kl;
    if pos >= len {
        return EOF_STATUS;
    }
    let b1 = *ptr.add(pos);
    let pos1 = pos + 1 + (b1.leading_ones() as usize);
    if pos1 >= len {
        return EOF_STATUS;
    }
    let b2 = *ptr.add(pos1);
    if b2 < 0x80 {
        (*out).framing_len = (pos1 + 1) as u64;
        (*out).promoted_len = b2 as u64;
        return 0;
    }
    wide_tail(ptr, len, pos1, out)
}
