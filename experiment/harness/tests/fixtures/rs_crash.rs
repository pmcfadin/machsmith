// Test fixture: a vint kernel that crashes when it has input.
#[no_mangle]
pub unsafe extern "C" fn msx_vint(ptr: *const u8, len: usize, out: *mut u64) -> usize {
    if len == 0 {
        return 0;
    }
    let _ = (ptr, out);
    core::ptr::null_mut::<u8>().write_volatile(1);
    0
}
