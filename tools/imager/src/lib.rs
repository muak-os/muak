//! Imager: shared deterministic image build pipeline.

#![warn(missing_docs)]

pub mod artifact;
pub mod build;
#[cfg(feature = "cli")]
pub mod cli;
pub mod error;
pub mod install;
pub mod profile;
pub mod request;
pub mod resolve;

mod catalog;
mod layout;
mod output;
mod render;
mod stage;
mod workspace;
