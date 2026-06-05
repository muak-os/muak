//! EFI System Partition manifest types and helpers.

#![warn(missing_docs)]
#![expect(
    clippy::pub_use,
    reason = "The crate intentionally exposes a flat public API at `esp::...`"
)]

extern crate alloc;

mod collect;
mod error;
mod format;
mod image;
mod model;
mod path;
mod populate;

pub use crate::{
    collect::collect_tree,
    error::EspError,
    format::format,
    image::build,
    model::{Arch, EspFile, EspSpec, EspSpecBuilder},
    populate::populate,
};
