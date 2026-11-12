//! Kernel `bti_sparse_find`: the `Sparse` arm of CQLite's `BtiNode::find_child`.

use crate::fold;

/// CQLite's `Transition { byte: u8, child: SizedPointer { distance: u64 } }`
/// with the pointer flattened to a `u64` and `#[repr(C)]` added (same 16-byte shape).
#[repr(C)]
pub struct Transition {
    pub byte: u8,
    pub child: u64,
}

/// Binary search for `byte` among sorted transitions; the body is the `Sparse`
/// arm of CQLite's `find_child`, with `child` being a plain `u64` here.
#[inline]
pub fn find_sparse(transitions: &[Transition], byte: u8) -> Option<&u64> {
    // BEGIN VERBATIM cqlite-core/src/storage/sstable/bti/node.rs :: BtiNode::find_child (Sparse arm) @ 2e5b7a4c
    transitions
        .binary_search_by_key(&byte, |t| t.byte)
        .ok()
        .map(|idx| &transitions[idx].child)
    // END VERBATIM
}

/// 1 if a transition with `.byte == (byte as u8)` exists (writes its child to
/// `*out`), else 0 (`*out` not written).
///
/// # Safety
/// `t` valid for `n` `Transition`s (or `n == 0`); `out` valid for a `u64` write.
#[no_mangle]
pub unsafe extern "C" fn msx_bti_sparse_find(t: *const Transition, n: usize, byte: u32, out: *mut u64) -> u32 {
    let transitions = core::slice::from_raw_parts(t, n);
    match find_sparse(transitions, byte as u8) {
        Some(child) => {
            *out = *child;
            1
        }
        None => 0,
    }
}

/// Walks records `[u32 LE n][u32 LE byte][n * 16 bytes of Transition]` from `seg`
/// (which must be 8-byte aligned). Stops if the remaining bytes cannot hold a
/// header or the `n * 16` payload. For each record folds the return value and,
/// when found, the child. Returns the number of records processed.
///
/// # Safety
/// `seg` valid for `seg_len` bytes and 8-byte aligned; `checksum` valid for a `u64` write.
#[no_mangle]
pub unsafe extern "C" fn msx_bti_sparse_find_chain(seg: *const u8, seg_len: usize, checksum: *mut u64) -> u64 {
    let mut off = 0usize;
    let mut acc = 0u64;
    let mut ops = 0u64;
    while seg_len - off >= 8 {
        let p = seg.add(off);
        let n = u32::from_le_bytes([*p, *p.add(1), *p.add(2), *p.add(3)]) as usize;
        let byte = u32::from_le_bytes([*p.add(4), *p.add(5), *p.add(6), *p.add(7)]);
        if seg_len - off - 8 < n * 16 {
            break;
        }
        let mut child = 0u64;
        let found = msx_bti_sparse_find(p.add(8) as *const Transition, n, byte, &mut child);
        acc = fold(acc, found as u64);
        if found != 0 {
            acc = fold(acc, child);
        }
        ops += 1;
        off += 8 + n * 16;
    }
    *checksum = acc;
    ops
}
