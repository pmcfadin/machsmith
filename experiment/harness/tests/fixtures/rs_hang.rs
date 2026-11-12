// Test fixture: a vint kernel that never returns when it has input.
#[no_mangle]
pub unsafe extern "C" fn msx_vint(ptr: *const u8, len: usize, out: *mut u64) -> usize {
    if len == 0 {
        return 0;
    }
    let mut x = core::ptr::read_volatile(ptr) as u64;
    loop {
        x = core::hint::black_box(x.wrapping_add(1));
        core::ptr::write_volatile(out, x);
    }
}
