//! Shared fixtures for the disk integration tests.

#![cfg(test)]

use disk::convert;
use disk::document::Document;
use disk::layout::Layout;
use disk::plan::Plan;

/// Returns the frozen system plan of the UEFI layout.
#[must_use]
pub fn uefi_plan() -> Plan {
    Layout::Uefi.plan()
}

/// Returns the document built from the frozen UEFI layout plan.
///
/// # Panics
///
/// Panics when the UEFI plan cannot be converted to a document.
#[must_use]
pub fn uefi_document() -> Document {
    convert::from_plan(&uefi_plan()).expect("doc from plan")
}
