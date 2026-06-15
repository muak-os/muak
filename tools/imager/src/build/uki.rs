//! UKI (Unified Kernel Image) building.

use std::io::{Cursor, Read as _};
use std::path::Path;

use tokio::task::spawn_blocking;
use yuki::section::Section;

use super::stage::{self, InstallerAssets};
use crate::error::{ImagerError, Result};

/// Build a UKI from installer assets and an initramfs tail blob.
///
/// Returns the UKI binary and the list of PE sections for TPM sealing.
///
/// # Errors
///
/// Returns an error when reading assets or building the UKI fails.
pub async fn uki(
    assets: &InstallerAssets,
    initramfs_tail: &[u8],
) -> Result<(Vec<u8>, Vec<Section>)> {
    let stub = stage::read_file(&assets.stub, "stub")?;
    let kernel = stage::read_file(&assets.kernel, "kernel")?;
    let cmdline = stage::read_file(&assets.cmdline, "cmdline")?;

    let base_file = assets.initramfs.clone();
    let base_len = base_file.len;
    let tail = initramfs_tail.to_vec();
    let initramfs_len = base_len.saturating_add(u64::try_from(tail.len()).unwrap_or(u64::MAX));

    let (uki_bytes, sections) = spawn_blocking(move || {
        let mut buf = Vec::new();
        let stub_len = u64::try_from(stub.len()).unwrap_or(u64::MAX);
        let kernel_len = u64::try_from(kernel.len()).unwrap_or(u64::MAX);
        let cmdline_len = u64::try_from(cmdline.len()).unwrap_or(u64::MAX);
        let mut stub_reader = Cursor::new(stub);
        let mut kernel_reader = Cursor::new(kernel);
        let mut cmdline_reader = Cursor::new(cmdline);

        let base_reader = base_file
            .open()
            .map_err(|e| ImagerError::BuildError(format!("open initramfs: {e}")))?;
        let tail_reader = Cursor::new(tail.as_slice());
        let mut initramfs_reader = base_reader.chain(tail_reader);

        let input = yuki::BuildInput {
            stub: yuki::SizedPart {
                len: stub_len,
                reader: &mut stub_reader,
            },
            kernel: yuki::SizedPart {
                len: kernel_len,
                reader: &mut kernel_reader,
            },
            initramfs: yuki::SizedPart {
                len: initramfs_len,
                reader: &mut initramfs_reader,
            },
            cmdline: yuki::SizedPart {
                len: cmdline_len,
                reader: &mut cmdline_reader,
            },
            dtb: None,
        };
        let sections = yuki::build(input, &mut buf)
            .map_err(|e| ImagerError::BuildError(format!("build UKI: {e}")))?;
        Ok::<_, ImagerError>((buf, sections))
    })
    .await
    .map_err(|e| ImagerError::BuildError(format!("join UKI build task: {e}")))??;

    Ok((uki_bytes, sections))
}

/// Writes a combined (base + tail) initramfs to a file on disk.
///
/// # Errors
///
/// Returns an error when reading the base initramfs or writing the file fails.
pub async fn write_initramfs(assets: &InstallerAssets, tail: &[u8], output: &Path) -> Result<()> {
    let base_file = assets.initramfs.clone();
    let tail = tail.to_vec();
    let output = output.to_path_buf();
    spawn_blocking(move || {
        let base_reader = base_file
            .open()
            .map_err(|e| ImagerError::BuildError(format!("open initramfs: {e}")))?;
        let tail_reader = Cursor::new(tail.as_slice());
        let mut combined = base_reader.chain(tail_reader);
        let file = std::fs::File::create(&output)
            .map_err(|e| ImagerError::BuildError(format!("create initramfs file: {e}")))?;
        let mut writer = std::io::BufWriter::new(file);
        std::io::copy(&mut combined, &mut writer)
            .map_err(|e| ImagerError::BuildError(format!("write initramfs: {e}")))?;
        Ok::<_, ImagerError>(())
    })
    .await
    .map_err(|e| ImagerError::BuildError(format!("join write initramfs task: {e}")))?
}
