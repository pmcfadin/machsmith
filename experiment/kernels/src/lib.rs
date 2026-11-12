//! Reference implementations of the three fixed kernels (`vint`,
//! `index_framing`, `bti_sparse_find`): CQLite's code copied verbatim behind the
//! experiment's C ABIs, plus the report-only `msx_<kernel>_chain` loops.
//!
//! The checksum for every chain is `acc = acc.rotate_left(5) ^ word`, folded over
//! each operation's output words in order, starting from 0.

pub mod bti_sparse_find;
pub mod index_framing;
pub mod traces;
pub mod vint;

/// One step of the chain checksum shared by every kernel.
#[inline(always)]
pub fn fold(acc: u64, word: u64) -> u64 {
    acc.rotate_left(5) ^ word
}
