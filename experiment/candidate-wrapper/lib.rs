#![cfg_attr(not(msx_train), no_std)]
mod candidate;
#[cfg(not(msx_train))]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    // A panicking candidate must crash, not hang: the checker reports it as OOB_OR_CRASH.
    unsafe { core::ptr::null_mut::<u8>().write_volatile(0) };
    loop {}
}
