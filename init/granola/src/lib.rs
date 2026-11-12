//! Service framework for granola-managed daemons.

pub mod runtime;

// A proc-macro can only be re-exported with `pub use`; there is no
// `pub extern crate` equivalent that lands in the macro namespace.
#[expect(
    clippy::useless_attribute,
    clippy::pub_use,
    reason = "re-exporting a proc-macro requires `pub use`"
)]
pub use granola_macros::service;
#[expect(
    clippy::useless_attribute,
    clippy::pub_use,
    reason = "re-exporting the kmsg crate for downstream users"
)]
pub use kmsg;
