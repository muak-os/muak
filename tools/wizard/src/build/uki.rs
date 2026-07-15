//! UKI assembly helper.

use std::io::Read as _;
use std::os::unix::net::UnixStream;

use koci::arch::Arch;
use sbolt::keys::SigningPair;
use sbolt::signature;
use tokio::task::{JoinHandle, block_in_place, spawn_blocking};
use yuki::builder::Builder;
use yuki::layout;
use yuki::pe::section::Section;

use super::archive::{TailParts, build_tail_from_parts};
use crate::error::{Result, WizardError};
use crate::source::installer::{self, Metadata};

/// A stream of UKI data, its size, and a handle for section metadata.
type UkiBuild = (UnixStream, u64, JoinHandle<Result<Vec<Section>>>);

/// Builds a UKI by streaming files from the installer image.
///
/// # Errors
///
/// Returns an error when yuki, signing, or tail building fails.
pub(crate) async fn build(
    installer_ref: &str,
    arch: &Arch,
    meta: &Metadata,
    tail_parts: &TailParts,
    tail_size: u64,
    signing_key: Option<&SigningPair<'_>>,
) -> Result<UkiBuild> {
    let initramfs_len = meta
        .initramfs_size
        .checked_add(tail_size)
        .ok_or_else(|| WizardError::BuildError("initramfs size overflow".to_owned()))?;

    let (stub_w, mut stub_r) = UnixStream::pair()
        .map_err(|e| WizardError::BuildError(format!("create stub pipe: {e}")))?;
    let (data_w, data_r) = UnixStream::pair()
        .map_err(|e| WizardError::BuildError(format!("create data pipe: {e}")))?;
    let (tail_w, tail_r) = UnixStream::pair()
        .map_err(|e| WizardError::BuildError(format!("create tail pipe: {e}")))?;
    let (unsigned_w, mut unsigned_r) = UnixStream::pair()
        .map_err(|e| WizardError::BuildError(format!("create unsigned pipe: {e}")))?;

    let installer_ref_owned = installer_ref.to_owned();
    let arch_copy = *arch;
    let installer_handle = tokio::spawn(async move {
        stream_installer_components(&installer_ref_owned, &arch_copy, stub_w, data_w).await
    });

    let (uki_layout, state) = block_in_place(|| {
        layout::compute(
            &mut stub_r,
            meta.stub_size,
            meta.cmdline_size,
            meta.kernel_size,
            initramfs_len,
            None,
        )
        .map_err(|e| WizardError::BuildError(format!("compute UKI layout: {e}")))
    })?;

    let sections_handle = spawn_blocking(move || {
        let mut stub_r = stub_r;
        let mut data_r = data_r;
        let tail_r = tail_r;
        let mut unsigned_w = unsigned_w;

        let builder = Builder::new(state, &mut unsigned_w);
        let builder = builder
            .add_stub(&mut stub_r)
            .map_err(|e| WizardError::BuildError(format!("add stub: {e}")))?;
        let builder = builder
            .add_cmdline(&mut data_r)
            .map_err(|e| WizardError::BuildError(format!("add cmdline: {e}")))?;
        let builder = builder
            .add_kernel(&mut data_r)
            .map_err(|e| WizardError::BuildError(format!("add kernel: {e}")))?;
        let mut initramfs_reader = data_r.chain(tail_r);
        let builder = builder
            .add_initramfs(&mut initramfs_reader)
            .map_err(|e| WizardError::BuildError(format!("add initramfs: {e}")))?;
        let sections = builder
            .finish()
            .map_err(|e| WizardError::BuildError(format!("finish UKI: {e}")))?;

        Ok::<_, WizardError>(sections)
    });

    block_in_place(|| {
        build_tail_from_parts(tail_parts, &mut &tail_w)
            .map_err(|e| WizardError::BuildError(format!("build tail: {e}")))
    })?;
    drop(tail_w);

    let output_r = if let Some(key) = signing_key {
        let (mut signed_w, signed_r) = UnixStream::pair()
            .map_err(|e| WizardError::BuildError(format!("create signed pipe: {e}")))?;
        block_in_place(|| {
            signature::sign(&mut unsigned_r, key.signer, key.certificate, &mut signed_w)
                .map_err(|e| WizardError::BuildError(format!("sign UKI: {e}")))
        })?;
        signed_r
    } else {
        unsigned_r
    };

    installer_handle
        .await
        .map_err(|e| WizardError::BuildError(format!("join installer task: {e}")))?
        .map_err(|e| WizardError::BuildError(format!("stream installer components: {e}")))?;

    Ok((output_r, uki_layout.total_size, sections_handle))
}

async fn stream_installer_components(
    installer_ref: &str,
    arch: &Arch,
    mut stub_w: UnixStream,
    mut data_w: UnixStream,
) -> Result<()> {
    installer::pull(installer_ref, arch, |path, _size, reader| {
        match path {
            "stub.efi" => {
                std::io::copy(reader, &mut stub_w)?;
            }
            "cmdline" | "vmlinuz" | "initramfs.img" => {
                std::io::copy(reader, &mut data_w)?;
            }
            _ => {}
        }
        Ok(())
    })
    .await
}
