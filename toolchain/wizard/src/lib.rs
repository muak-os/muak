//! Wizard: shared deterministic image build pipeline.

#![warn(missing_docs)]

extern crate alloc;

pub mod arch;
pub mod artifact;
pub mod codec;
pub mod config;
pub mod domain;
pub mod error;
mod nodes;
mod pipeline;
pub mod request;
pub mod resolver;
mod stream;

use uki::measure::MeasuredSection;

/// Artifact build metadata.
#[derive(Debug, Default)]
pub struct Metadata {
    /// PE section measurement records for the built UKI.
    pub sections: Vec<MeasuredSection>,
}
