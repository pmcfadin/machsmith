//! Loading a shared object and resolving kernel symbols. Libraries are never `dlclose`d.

use crate::spec::RawFn;
use std::ffi::{CStr, CString};
use std::path::Path;

pub struct Lib {
    handle: *mut libc::c_void,
}

// The handle is an opaque, process-lifetime dlopen handle; dlsym is thread-safe.
unsafe impl Send for Lib {}
unsafe impl Sync for Lib {}

impl Lib {
    pub fn open(path: &Path) -> Result<Lib, String> {
        let c = CString::new(path.as_os_str().as_encoded_bytes().to_vec())
            .map_err(|_| format!("path contains NUL: {}", path.display()))?;
        let handle = unsafe { libc::dlopen(c.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
        if handle.is_null() {
            return Err(dl_error().unwrap_or_else(|| format!("dlopen {} failed", path.display())));
        }
        Ok(Lib { handle })
    }

    pub fn sym(&self, name: &str) -> Option<RawFn> {
        let c = CString::new(name).ok()?;
        let p = unsafe { libc::dlsym(self.handle, c.as_ptr()) };
        if p.is_null() {
            None
        } else {
            Some(p as RawFn)
        }
    }
}

fn dl_error() -> Option<String> {
    let e = unsafe { libc::dlerror() };
    if e.is_null() {
        None
    } else {
        Some(unsafe { CStr::from_ptr(e) }.to_string_lossy().into_owned())
    }
}
