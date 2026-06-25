//! UKI assembly helper.

use std::io::Read;
use std::io::Write;
use std::os::unix::net::UnixStream;

use ring::digest;
use sbolt::keys::SigningPair;
use sbolt::signature;
use tokio::task::{block_in_place, spawn_blocking};
use yuki::BuildInput;
use yuki::SizedPart;
use yuki::section::Section;

use super::archive::{TailParts, build_tail_from_parts};
use super::stage::InstallerAssets;
use crate::error::{Result, WizardError};

/// Builds a UKI from prepulled installer assets and prebuilt tail parts.
///
/// # Errors
///
/// Returns an error when yuki or signing fails.
pub(crate) async fn build_uki(
    assets: &InstallerAssets,
    tail_parts: &TailParts,
    tail_size: u64,
    signing_key: Option<&SigningPair<'_>>,
    uki_writer: &mut impl Write,
) -> Result<(Vec<Section>, Vec<[u8; 32]>)> {
    let kernel_file = assets.kernel.clone();
    let stub_file = assets.stub.clone();
    let cmdline_file = assets.cmdline.clone();

    let cmdline_hash = sha256(&[&assets.cmdline.data]);
    let kernel_hash = sha256(&[&assets.kernel.data]);

    let base_file = assets.initramfs.clone();
    let base_len = base_file.len;
    let initramfs_len = base_len.saturating_add(tail_size);
    let stub_len = assets.stub.len;
    let kernel_len = assets.kernel.len;
    let cmdline_len = assets.cmdline.len;

    let (tail_w, tail_r) = UnixStream::pair()
        .map_err(|e| WizardError::BuildError(format!("create tail pipe: {e}")))?;
    let (uki_w, mut uki_r) =
        UnixStream::pair().map_err(|e| WizardError::BuildError(format!("create UKI pipe: {e}")))?;

    let sections_handle = spawn_blocking(move || {
        let mut stub_reader = stub_file
            .open()
            .map_err(|e| WizardError::BuildError(format!("open stub: {e}")))?;
        let mut kernel_reader = kernel_file
            .open()
            .map_err(|e| WizardError::BuildError(format!("open kernel: {e}")))?;
        let mut cmdline_reader = cmdline_file
            .open()
            .map_err(|e| WizardError::BuildError(format!("open cmdline: {e}")))?;
        let base_reader = base_file
            .open()
            .map_err(|e| WizardError::BuildError(format!("open initramfs: {e}")))?;
        let mut initramfs_reader = HashReader {
            inner: base_reader.chain(tail_r),
            ctx: digest::Context::new(&digest::SHA256),
        };

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
        let sections = yuki::build(input, &mut &uki_w)
            .map_err(|e| WizardError::BuildError(format!("build UKI: {e}")))?;
        let digest = initramfs_reader.ctx.finish();
        let mut initrd_hash = [0; 32];
        initrd_hash.copy_from_slice(digest.as_ref());

        Ok::<_, WizardError>((sections, initrd_hash))
    });

    block_in_place(|| {
        build_tail_from_parts(tail_parts, &mut &tail_w)
            .map_err(|e| WizardError::BuildError(format!("build tail: {e}")))
    })?;

    if let Some(key) = signing_key {
        signature::sign(&mut uki_r, key.signer, key.certificate, uki_writer)
            .map_err(|e| WizardError::BuildError(format!("sign UKI: {e}")))?;
    } else {
        std::io::copy(&mut uki_r, uki_writer)
            .map_err(|e| WizardError::BuildError(format!("read UKI pipe: {e}")))?;
    }

    let (sections, initrd_hash) = sections_handle
        .await
        .map_err(|e| WizardError::BuildError(format!("join UKI build task: {e}")))??;

    let mut section_hashes = Vec::with_capacity(sections.len());
    for section in &sections {
        let hash = match section.name {
            ".cmdline" => cmdline_hash,
            ".linux" => kernel_hash,
            ".initrd" => initrd_hash,
            n => return Err(WizardError::BuildError(format!("unexpected section {n}"))),
        };
        section_hashes.push(hash);
    }

    Ok((sections, section_hashes))
}

struct HashReader<R> {
    inner: R,
    ctx: digest::Context,
}

impl<R: Read> Read for HashReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.ctx.update(buf.get(..n).unwrap_or(&[]));

        Ok(n)
    }
}

fn sha256(parts: &[&[u8]]) -> [u8; 32] {
    let mut ctx = digest::Context::new(&digest::SHA256);
    for part in parts {
        ctx.update(part);
    }
    let digest = ctx.finish();
    let mut hash = [0; 32];
    hash.copy_from_slice(digest.as_ref());

    hash
}
