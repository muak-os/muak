//! Interact with TPM2 interface via /dev/tpmrm0.

mod commands;
mod device;
pub mod errors;
mod operations;
pub mod pcr;
mod types;

pub use device::is_available;
pub use errors::{Error, Result};
pub use operations::{SealedBlob, seal, seal_to_pcr11, unseal, unseal_from_blob};
pub use pcr::{compute_policy_digest, predict_pcr11};
pub use types::SHA256_DIGEST_SIZE;
