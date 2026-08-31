//! Service framework for granola-managed daemons.

pub use granola_macros::service;
pub use kmsg;

mod runtime;

pub use runtime::{Health, NotifyClient, shutdown_signal, socket};
