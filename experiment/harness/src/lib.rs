//! `msx-harness`: checks and times LLM-written assembly against rustc builds of CQLite kernels.

pub mod build;
pub mod check;
pub mod guard;
pub mod lint;
pub mod loader;
pub mod spec;
pub mod spec_bti_sparse_find;
pub mod spec_index_framing;
pub mod spec_vint;
pub mod tramp;

pub use spec::{all_kernels, kernel, kernels_with_traces, KernelSpec, OutRec, RawFn};
pub use tramp::AbiViolation;
