//! Wizard: shared deterministic image build pipeline.

#![warn(missing_docs)]

extern crate alloc;

pub mod arch;
pub mod artifact;
#[cfg(feature = "cli")]
pub mod cli;
pub mod config;
pub mod domain;
pub mod error;
mod nodes;
mod pipeline;
pub mod request;
pub(crate) mod resolver;
mod stream;

use serde::{Deserialize, Serialize};

/// Artifact build metadata.
#[derive(Debug, Default)]
pub struct Metadata {
    /// PE section descriptors for the built UKI.
    pub sections: Vec<SectionInfo>,
}

/// PE section metadata needed for TPM PCR#11 prediction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectionInfo {
    /// PE section name.
    pub name: String,
    /// File offset of the section data within the PE image.
    pub file_offset: usize,
    /// Size of the section data in bytes.
    pub size: usize,
    /// SHA-256 hash of the section data.
    pub hash: [u8; 32],
}
