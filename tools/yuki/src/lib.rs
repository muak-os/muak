//! Yuki - A library to create Unified Kernel Images (UKI) for Linux on UEFI systems.

#![warn(missing_docs)]

mod align;
mod assembler;
#[cfg(feature = "cli")]
pub mod cli;
pub mod error;
mod pe;
mod prefix;
pub mod section;
mod stream;

use std::io::{Read, Write};

use error::Result;

/// A readable UKI component with an exact byte length.
pub struct SizedPart<'a> {
    /// Exact component length in bytes.
    pub len: u64,
    /// Readable stream for the component bytes.
    pub reader: &'a mut dyn Read,
}

/// Component data required to build a Unified Kernel Image.
pub struct BuildInput<'a> {
    /// EFI stub PE binary to embed components into.
    pub stub: SizedPart<'a>,
    /// Kernel image.
    pub kernel: SizedPart<'a>,
    /// Initial RAM filesystem image.
    pub initramfs: SizedPart<'a>,
    /// Kernel command-line string.
    pub cmdline: SizedPart<'a>,
    /// Optional device-tree blob.
    pub dtb: Option<SizedPart<'a>>,
}

/// Builds a Unified Kernel Image by embedding components into an EFI stub.
///
/// Returns section metadata.
///
/// # Errors
///
/// Returns an error if the stub is not a valid PE image, the section count
/// exceeds the PE limit, or writing the output fails.
pub fn build<W: Write>(input: BuildInput<'_>, writer: &mut W) -> Result<Vec<section::Section>> {
    assembler::assemble(input, writer)
}
