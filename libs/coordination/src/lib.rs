//! Transport-independent foundation for optional Muak fleet inventory.
//!
//! This draft implements an in-memory registry, credential bindings, bounded
//! snapshots and session replacement. The caller supplies identities obtained
//! from authenticated transport and approved by an administrator. This crate
//! does not verify certificates, persist enrollment, expose a network service,
//! or authorize workload mutations.
//!
//! See the proposed [architecture decision record][adr] for the delivery scope.
//!
//! [adr]: https://github.com/muak-os/muak/blob/feat/star-coordination/docs/adr/0001-multi-host-coordination.md

#![warn(missing_docs)]

extern crate alloc;

pub mod error;
pub mod inventory;
pub mod registry;
