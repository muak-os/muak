//! Kernel module management library.
//!
//! Provides utilities for loading kernel modules, resolving dependencies,
//! and managing module aliases.

mod alias;
mod dep;
mod discovery;
mod loader;

pub use alias::AliasDb;
pub use dep::DepDb;
pub use discovery::for_each_modalias;
pub use loader::{LoadError, ModuleLoader, load_module};
