//! UKI assembly helper.

use std::io::Read as _;
use std::os::unix::net::UnixStream;

use sbolt::keys::SigningPair;
use sbolt::signature;
use tokio::task::{JoinHandle, block_in_place, spawn_blocking};
use yuki::BuildInput;
use yuki::SizedPart;
use yuki::section::Section;

use super::archive::{TailParts, build_tail_from_parts};
use super::source::InstallerAssets;
use crate::error::{Result, WizardError};

/// A stream of UKI data, its size, and a handle for section metadata.
type UkiBuild = (
    std::os::unix::net::UnixStream,
    u64,
    JoinHandle<Result<Vec<Section>>>,
);

/// Builds a UKI from prepulled installer assets and prebuilt tail parts.
///
/// # Errors
///
/// Returns an error when yuki, signing, or tail building fails.
pub(crate) fn build(
    assets: &InstallerAssets,
    tail_parts: &TailParts,
    tail_size: u64,
    signing_key: Option<&SigningPair<'_>>,
) -> Result<UkiBuild> {
    let stub_len = assets.stub.len;
    let kernel_len = assets.kernel.len;
    let cmdline_len = assets.cmdline.len;
    let base_len = assets.initramfs.len;
    let initramfs_len = base_len.saturating_add(tail_size);

    let mut stub_reader = assets.stub.open();
    let uki_size = yuki::compute_size(
        &mut stub_reader,
        stub_len,
        cmdline_len,
        kernel_len,
        initramfs_len,
        None,
    )
    .map_err(|e| WizardError::BuildError(format!("compute UKI size: {e}")))?;
    drop(stub_reader);

    let (tail_w, tail_r) = UnixStream::pair()
        .map_err(|e| WizardError::BuildError(format!("create tail pipe: {e}")))?;
    let (signed_w, signed_r) =
        UnixStream::pair().map_err(|e| WizardError::BuildError(format!("create UKI pipe: {e}")))?;

    let stub_file = assets.stub.clone();
    let kernel_file = assets.kernel.clone();
    let cmdline_file = assets.cmdline.clone();
    let base_file = assets.initramfs.clone();

    let (unsigned_w, mut unsigned_r) = UnixStream::pair()
        .map_err(|e| WizardError::BuildError(format!("create unsigned pipe: {e}")))?;

    let sections_handle = spawn_blocking(move || {
        let mut stub_reader = stub_file.open();
        let mut kernel_reader = kernel_file.open();
        let mut cmdline_reader = cmdline_file.open();
        let base_reader = base_file.open();
        let mut initramfs_reader = base_reader.chain(tail_r);

        let input = BuildInput {
            stub: SizedPart {
                len: stub_len,
                reader: &mut stub_reader,
            },
            kernel: SizedPart {
                len: kernel_len,
                reader: &mut kernel_reader,
            },
            initramfs: SizedPart {
                len: initramfs_len,
                reader: &mut initramfs_reader,
            },
            cmdline: SizedPart {
                len: cmdline_len,
                reader: &mut cmdline_reader,
            },
            dtb: None,
        };
        let sections = yuki::build(input, &mut &unsigned_w)
            .map_err(|e| WizardError::BuildError(format!("build UKI: {e}")))?;

        Ok::<_, WizardError>(sections)
    });

    block_in_place(|| {
        build_tail_from_parts(tail_parts, &mut &tail_w)
            .map_err(|e| WizardError::BuildError(format!("build tail: {e}")))
    })?;
    drop(tail_w);

    if let Some(key) = signing_key {
        signature::sign(&mut unsigned_r, key.signer, key.certificate, &mut &signed_w)
            .map_err(|e| WizardError::BuildError(format!("sign UKI: {e}")))?;
    } else {
        std::io::copy(&mut unsigned_r, &mut &signed_w)
            .map_err(|e| WizardError::BuildError(format!("pipe UKI: {e}")))?;
    }
    drop(signed_w);

    Ok((signed_r, uki_size, sections_handle))
}
