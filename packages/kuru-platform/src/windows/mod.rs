//! Audited Windows interop. Public operations own their handles and allocations;
//! callers never supply unvalidated raw pointers or assume a PID grants ownership.

pub mod console;
pub mod pipe;
pub mod process;
pub(crate) mod security;
