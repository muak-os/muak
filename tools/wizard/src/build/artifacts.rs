//! Artifact build orchestration helpers.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;

use esp::FileMeta;
use sbolt::keys::SigningPair;
use tokio::task::JoinHandle;
use yuki::pe::section::Section;

use super::archive;
use super::archive::TailParts;
use super::media;
use super::uki;
use crate::error::{Result, WizardError};
use crate::resolve::BuildPlan;
use crate::source::{installer, overlay};

/// Pipes for streaming overlay components.
struct OverlayPipes {
    files: Vec<FileMeta<'static>>,
    readers: Vec<UnixStream>,
    handle: JoinHandle<Result<()>>,
}

/// Sets up pipes for streaming overlay components.
async fn setup_overlay_pipes(overlay: &overlay::Overlay) -> Result<OverlayPipes> {
    let files = overlay::metadata(overlay).await?;
    let pipe_pairs: Vec<(UnixStream, UnixStream)> = files
        .iter()
        .map(|_| {
            UnixStream::pair()
                .map_err(|e| WizardError::BuildError(format!("create overlay pipe: {e}")))
        })
        .collect::<Result<Vec<_>>>()?;

    let readers: Vec<UnixStream> = pipe_pairs
        .iter()
        .map(|pair| {
            pair.0
                .try_clone()
                .map_err(|e| WizardError::BuildError(format!("clone overlay pipe: {e}")))
        })
        .collect::<Result<Vec<_>>>()?;

    let writers: Vec<UnixStream> = pipe_pairs
        .into_iter()
        .map(|(_reader, writer)| writer)
        .collect();

    let path_to_index: HashMap<&str, usize> = files
        .iter()
        .enumerate()
        .map(|(i, meta)| (meta.path, i))
        .collect();

    let overlay_info = overlay.clone();
    let handle = tokio::spawn(async move {
        stream_overlay_components(&overlay_info, &path_to_index, writers).await
    });

    Ok(OverlayPipes {
        files,
        readers,
        handle,
    })
}

/// Configuration for [`build_post`].
pub(crate) struct BuildPostConfig<'a> {
    pub resolved: &'a BuildPlan,
    pub installer_meta: &'a installer::Metadata,
    pub tail_parts: &'a TailParts,
    pub tail_size: u64,
    pub signing_key: Option<&'a SigningPair<'a>>,
}

/// Builds all requested artifacts with a single installer pull.
///
/// This function handles all artifact types (UKI, ISO, Raw, kernel, cmdline, initramfs)
/// in a single pass, eliminating redundant installer pulls.
pub(crate) async fn build<W: Write>(
    config: &BuildPostConfig<'_>,
    uki: Option<&mut W>,
    iso: Option<&mut W>,
    raw: Option<&mut W>,
    kernel: Option<&mut W>,
    cmdline: Option<&mut W>,
    initramfs: Option<&mut W>,
) -> Result<Vec<Section>> {
    let needs_uki = uki.is_some() || iso.is_some() || raw.is_some();
    let needs_media = iso.is_some() || raw.is_some();

    // Set up overlay pipes only if we need media artifacts
    let mut overlay_pipes = if needs_media {
        match config.resolved.overlay() {
            Some(info) => Some(setup_overlay_pipes(info).await?),
            None => None,
        }
    } else {
        None
    };

    // Set up UKI pipes only if we need UKI artifacts
    let mut uki_build = if needs_uki {
        Some(uki::build(
            config.installer_meta,
            config.tail_size,
            config.signing_key,
        )?)
    } else {
        None
    };

    // Write tail to UKI pipes if we have them
    if let Some(ref mut build) = uki_build {
        uki::write_tail(build, config.tail_parts)?;
    }

    // Single pull with tee to all consumers
    let (uki_stub_w, uki_data_w) = if let Some(ref mut build) = uki_build {
        (Some(&mut build.stub_w), Some(&mut build.data_w))
    } else {
        (None, None)
    };

    pull_and_tee::<W>(
        config.resolved,
        uki_stub_w,
        uki_data_w,
        kernel,
        cmdline,
        initramfs,
        needs_uki.then_some(config.tail_parts),
    )
    .await?;

    // Finalize UKI if we built it
    let sections = if let Some(uki_build) = uki_build {
        let (mut uki_reader, uki_size, sections_handle) = uki::collect(uki_build);

        let empty_files = Vec::<FileMeta<'_>>::new();
        let (overlay_files, mut overlay_reader_refs) = if let Some(ref mut pipes) = overlay_pipes {
            let files = pipes.files.as_slice();
            let readers: Vec<&mut dyn Read> = pipes
                .readers
                .iter_mut()
                .map::<&mut dyn Read, _>(|reader| reader)
                .collect();
            (files, readers)
        } else {
            (empty_files.as_slice(), Vec::new())
        };

        let overlay_reader_slice = if overlay_reader_refs.is_empty() {
            None
        } else {
            Some(&mut *overlay_reader_refs)
        };

        if let Some(w) = iso {
            media::write(
                config.resolved.arch(),
                &mut uki_reader,
                uki_size,
                overlay_files,
                overlay_reader_slice,
                Some(w),
                None,
            )?;
        } else if let Some(w) = raw {
            media::write(
                config.resolved.arch(),
                &mut uki_reader,
                uki_size,
                overlay_files,
                overlay_reader_slice,
                None,
                Some(w),
            )?;
        } else if let Some(w) = uki {
            std::io::copy(&mut uki_reader, w)
                .map_err(|e| WizardError::BuildError(format!("write UKI: {e}")))?;
        } else {
            // No output requested
        }

        sections_handle
            .await
            .map_err(|e| WizardError::BuildError(format!("join UKI build task: {e}")))?
    } else {
        Ok(Vec::default())
    }?;

    // Wait for overlay streaming to complete
    if let Some(pipes) = overlay_pipes {
        pipes
            .handle
            .await
            .map_err(|e| WizardError::BuildError(format!("join overlay task: {e}")))?
            .map_err(|e| WizardError::BuildError(format!("stream overlay components: {e}")))?;
    }

    Ok(sections)
}

async fn stream_overlay_components(
    info: &overlay::Overlay,
    path_to_index: &HashMap<&str, usize>,
    mut writers: Vec<UnixStream>,
) -> Result<()> {
    overlay::pull(info, |path, _size, reader| {
        let Some(&idx) = path_to_index.get(path) else {
            return Ok(());
        };
        let writer = writers.get_mut(idx).ok_or_else(|| {
            WizardError::BuildError(format!("overlay file index {idx} out of range"))
        })?;
        std::io::copy(reader, writer)
            .map_err(|e| WizardError::BuildError(format!("stream overlay file: {e}")))?;
        Ok(())
    })
    .await
}

fn copy_if_some<W: Write>(
    reader: &mut dyn Read,
    writer: &mut Option<&mut W>,
) -> std::io::Result<()> {
    if let Some(w) = writer.as_deref_mut() {
        std::io::copy(reader, w)?;
    }

    Ok(())
}

struct TeeTargets<'a, W: Write> {
    uki_stub_w: Option<&'a mut UnixStream>,
    uki_data_w: Option<&'a mut UnixStream>,
    kernel: Option<&'a mut W>,
    cmdline: Option<&'a mut W>,
    initramfs: Option<&'a mut W>,
    tail_parts: Option<&'a TailParts>,
}

impl<W: Write> TeeTargets<'_, W> {
    fn tee_component(&mut self, path: &str, reader: &mut dyn Read) -> std::io::Result<()> {
        match path {
            "stub.efi" => copy_if_some(reader, &mut self.uki_stub_w),
            "cmdline" => {
                copy_if_some(reader, &mut self.uki_data_w)?;
                copy_if_some(reader, &mut self.cmdline)
            }
            "vmlinuz" => {
                copy_if_some(reader, &mut self.uki_data_w)?;
                copy_if_some(reader, &mut self.kernel)
            }
            "initramfs.img" => {
                copy_if_some(reader, &mut self.uki_data_w)?;
                self.write_initramfs_with_tail(reader)
            }
            _ => Ok(()),
        }
    }

    fn write_initramfs_with_tail(&mut self, reader: &mut dyn Read) -> std::io::Result<()> {
        let Some(ref mut writer) = self.initramfs else {
            return Ok(());
        };
        std::io::copy(reader, writer)?;
        if let Some(tail) = self.tail_parts {
            archive::build_tail_from_parts(tail, writer).map_err(std::io::Error::other)?;
        }

        Ok(())
    }
}

async fn pull_and_tee<W: Write>(
    resolved: &BuildPlan,
    uki_stub_w: Option<&mut UnixStream>,
    uki_data_w: Option<&mut UnixStream>,
    kernel: Option<&mut W>,
    cmdline: Option<&mut W>,
    initramfs: Option<&mut W>,
    tail_parts: Option<&TailParts>,
) -> Result<()> {
    let mut targets = TeeTargets {
        uki_stub_w,
        uki_data_w,
        kernel,
        cmdline,
        initramfs,
        tail_parts,
    };

    installer::pull(
        resolved.installer(),
        &resolved.arch(),
        |path, _size, reader| targets.tee_component(path, reader),
    )
    .await
}
