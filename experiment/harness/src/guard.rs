//! Guard-page buffers: inputs placed flush against an inaccessible page so any
//! out-of-bounds read faults, and a sized output slot whose overrun faults.

const PAGE: usize = 4096;

unsafe fn map(len: usize) -> *mut u8 {
    let p = libc::mmap(
        core::ptr::null_mut(),
        len,
        libc::PROT_READ | libc::PROT_WRITE,
        libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
        -1,
        0,
    );
    assert!(p != libc::MAP_FAILED, "mmap failed");
    p as *mut u8
}

unsafe fn protect_none(p: *mut u8, len: usize) {
    assert_eq!(libc::mprotect(p as *mut libc::c_void, len, libc::PROT_NONE), 0, "mprotect failed");
}

/// `[PROT_NONE page][rw pages][PROT_NONE page]`.
pub struct GuardBuf {
    base: *mut u8,
    rw_pages: usize,
}

unsafe impl Send for GuardBuf {}

impl GuardBuf {
    pub fn new(max_len: usize) -> GuardBuf {
        let rw_pages = max_len.div_ceil(PAGE).max(1);
        unsafe {
            let base = map((rw_pages + 2) * PAGE);
            protect_none(base, PAGE);
            protect_none(base.add((rw_pages + 1) * PAGE), PAGE);
            GuardBuf { base, rw_pages }
        }
    }

    fn rw_start(&self) -> *mut u8 {
        unsafe { self.base.add(PAGE) }
    }

    fn rw_len(&self) -> usize {
        self.rw_pages * PAGE
    }

    /// Last byte sits immediately before the trailing guard page. The rest of the
    /// writable area is refilled with a poison byte.
    pub fn place_end(&mut self, bytes: &[u8]) -> *const u8 {
        assert!(bytes.len() <= self.rw_len(), "GuardBuf too small");
        unsafe {
            core::ptr::write_bytes(self.rw_start(), 0xA5, self.rw_len());
            let dst = self.rw_start().add(self.rw_len() - bytes.len());
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), dst, bytes.len());
            dst
        }
    }

    /// First byte sits immediately after the leading guard page.
    pub fn place_start(&mut self, bytes: &[u8]) -> *const u8 {
        assert!(bytes.len() <= self.rw_len(), "GuardBuf too small");
        unsafe {
            core::ptr::write_bytes(self.rw_start(), 0xA5, self.rw_len());
            let dst = self.rw_start();
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), dst, bytes.len());
            dst
        }
    }
}

impl Drop for GuardBuf {
    fn drop(&mut self) {
        unsafe {
            libc::munmap(self.base as *mut libc::c_void, (self.rw_pages + 2) * PAGE);
        }
    }
}

pub const SENTINEL: u64 = 0xDEAD_BEEF_DEAD_BEEF;

/// Exactly `words * 8` writable bytes ending at a guard page.
pub struct OutSlot {
    base: *mut u8,
    ptr: *mut u64,
    words: usize,
}

unsafe impl Send for OutSlot {}

impl OutSlot {
    pub fn new(words: usize) -> OutSlot {
        assert!((1..=3).contains(&words), "OutSlot holds 1..=3 words");
        unsafe {
            let base = map(2 * PAGE);
            protect_none(base.add(PAGE), PAGE);
            let ptr = base.add(PAGE - words * 8) as *mut u64;
            OutSlot { base, ptr, words }
        }
    }

    /// Fills with the sentinel and returns the pointer to hand to the kernel.
    pub fn arm(&mut self) -> *mut u64 {
        for i in 0..self.words {
            unsafe { self.ptr.add(i).write_volatile(SENTINEL) };
        }
        self.ptr
    }

    /// Values (unused words zero) and whether any word differs from the sentinel.
    pub fn take(&self) -> ([u64; 3], bool) {
        let mut v = [0u64; 3];
        let mut changed = false;
        for i in 0..self.words.min(3) {
            let w = unsafe { self.ptr.add(i).read_volatile() };
            v[i] = w;
            changed |= w != SENTINEL;
        }
        (v, changed)
    }
}

impl Drop for OutSlot {
    fn drop(&mut self) {
        unsafe {
            libc::munmap(self.base as *mut libc::c_void, 2 * PAGE);
        }
    }
}

thread_local! {
    static SLOTS: core::cell::RefCell<[Option<OutSlot>; 3]> = const { core::cell::RefCell::new([None, None, None]) };
}

/// Runs `f` with this thread's cached `OutSlot` of `words` words (created on first use), so the
/// per-call mmap/mprotect/munmap of `OutSlot::new` is paid once. Callers re-`arm` it per call.
pub fn with_slot<R>(words: usize, f: impl FnOnce(&mut OutSlot) -> R) -> R {
    assert!((1..=3).contains(&words));
    SLOTS.with(|c| {
        let mut slots = c.borrow_mut();
        let slot = slots[words - 1].get_or_insert_with(|| OutSlot::new(words));
        f(slot)
    })
}
