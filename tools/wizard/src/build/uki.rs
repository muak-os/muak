use std::io::Read as _;
use std::os::unix::net::UnixStream;

use sbolt::keys::SigningPair;
use sbolt::signature;
use tokio::task::{JoinHandle, block_in_place, spawn_blocking};
use yuki::builder::Builder;
use yuki::layout;
use yuki::pe::section::Section;

use super::archive::{TailParts, build_tail_from_parts};
use crate::error::{Result, WizardError};
use crate::source::installer::Metadata;

/// A UKI build session.
pub(crate) struct Build {
    pub stub_w: UnixStream,
    pub data_w: UnixStream,
    pub tail_w: UnixStream,
    pub output_r: UnixStream,
    pub total_size: u64,
    pub sections_handle: JoinHandle<Result<Vec<Section>>>,
}

/// Creates the UKI assembly pipeline and returns a [`Build`] session.
pub(crate) fn build(
    meta: &Metadata,
    tail_size: u64,
    signing_key: Option<&SigningPair<'_>>,
) -> Result<Build> {
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
        let mut unsigned_w = unsigned_w;
        let mut stub_r = stub_r;
        let mut data_r = data_r;

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

        builder
            .finish()
            .map_err(|e| WizardError::BuildError(format!("finish UKI: {e}")))
    });

    let output_r = match signing_key {
        Some(key) => create_signed_output(&mut unsigned_r, key)?,
        None => unsigned_r,
    };

    Ok(Build {
        stub_w,
        data_w,
        tail_w,
        output_r,
        total_size: uki_layout.total_size,
        sections_handle,
    })
}

/// Writes the initramfs tail archive into the tail pipe.
pub(crate) fn write_tail(build: &mut Build, tail_parts: &TailParts) -> Result<()> {
    let mut tail_w = build
        .tail_w
        .try_clone()
        .map_err(|e| WizardError::BuildError(format!("clone tail pipe: {e}")))?;

    block_in_place(|| {
        build_tail_from_parts(tail_parts, &mut tail_w)
            .map_err(|e| WizardError::BuildError(format!("write tail: {e}")))
    })
}

/// Closes the write ends and returns the output stream, total size, and
/// a handle to resolve the PE section metadata.
pub(crate) fn collect(build: Build) -> (UnixStream, u64, JoinHandle<Result<Vec<Section>>>) {
    drop(build.stub_w);
    drop(build.data_w);
    drop(build.tail_w);

    (build.output_r, build.total_size, build.sections_handle)
}

fn create_signed_output(unsigned_r: &mut UnixStream, key: &SigningPair<'_>) -> Result<UnixStream> {
    let (mut signed_w, signed_r) = UnixStream::pair()
        .map_err(|e| WizardError::BuildError(format!("create signed pipe: {e}")))?;
    block_in_place(|| {
        signature::sign(unsigned_r, key.signer, key.certificate, &mut signed_w)
            .map_err(|e| WizardError::BuildError(format!("sign UKI: {e}")))
    })?;

    Ok(signed_r)
}
