//! PE/COFF Authenticode signing.

mod authenticode;
mod signature;

pub use authenticode::compute_hash;
pub use signature::sign;
