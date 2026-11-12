// Sabotage candidate (Rust): contains core::arch::asm!. The source lint must reject it.

#[no_mangle]
pub unsafe extern "C" fn msx_vint(ptr: *const u8, len: usize, out: *mut u64) -> usize {
    if len == 0 {
        return 0;
    }
    let mut v: u64 = *ptr as u64;
    core::arch::asm!("nop", options(nomem, nostack));
    if v >= 0x80 {
        return 0;
    }
    v &= 0x7F;
    *out = v;
    1
}
