//! PE/COFF Authenticode signing.

mod authenticode;
mod signature;

pub use authenticode::compute_hash;
pub use signature::sign;

pub const PE32_PLUS_MAGIC: u16 = 0x20b;
