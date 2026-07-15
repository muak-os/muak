//! Artifact build orchestration helpers.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;

use sbolt::keys::SigningPair;
use yuki::pe::section::Section;

use super::archive;
use super::archive::TailParts;
use super::media;
use super::uki;
use crate::error::{Result, WizardError};
use crate::resolve::BuildPlan;
use crate::source::{installer, overlay};

#[expect(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    clippy::excessive_nesting,
    reason = "internal function taking post-processing inputs"
)]
pub(crate) async fn build_post<W: Write>(
    resolved: &BuildPlan,
    installer_meta: &installer::Metadata,
    tail_parts: &TailParts,
    tail_size: u64,
    signing_key: Option<&SigningPair<'_>>,
    uki: Option<&mut W>,
    iso: Option<&mut W>,
    raw: Option<&mut W>,
) -> Result<Vec<Section>> {
    let needs_media = iso.is_some() || raw.is_some();

    let (overlay_files, mut overlay_readers, overlay_handle) = if needs_media {
        match resolved.overlay() {
            Some(info) => {
                let files = overlay::metadata(info).await?;
                let pipe_pairs: Vec<(UnixStream, UnixStream)> = files
                    .iter()
                    .map(|_| {
                        UnixStream::pair().map_err(|e| {
                            WizardError::BuildError(format!("create overlay pipe: {e}"))
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;

                let readers: Vec<UnixStream> = pipe_pairs
                    .iter()
                    .map(|pair| {
                        pair.0.try_clone().map_err(|e| {
                            WizardError::BuildError(format!("clone overlay pipe: {e}"))
                        })
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

                let overlay_info = info.clone();
                let handle = tokio::spawn(async move {
                    stream_overlay_components(&overlay_info, &path_to_index, writers).await
                });

                (files, readers, handle)
            }
            None => (
                Vec::new(),
                Vec::new(),
                tokio::spawn(async { Ok::<(), WizardError>(()) }),
            ),
        }
    } else {
        (
            Vec::new(),
            Vec::new(),
            tokio::spawn(async { Ok::<(), WizardError>(()) }),
        )
    };

    let (mut uki_reader, uki_size, sections_handle) = uki::build(
        resolved.installer(),
        &resolved.arch(),
        installer_meta,
        tail_parts,
        tail_size,
        signing_key,
    )
    .await?;

    let mut overlay_reader_refs: Vec<&mut dyn Read> = overlay_readers
        .iter_mut()
        .map(|reader| -> &mut dyn Read { reader })
        .collect();
    let overlay_reader_slice = if overlay_reader_refs.is_empty() {
        None
    } else {
        Some(&mut *overlay_reader_refs)
    };

    if let Some(w) = iso {
        media::write(
            resolved.arch(),
            &mut uki_reader,
            uki_size,
            &overlay_files,
            overlay_reader_slice,
            Some(w),
            None,
        )?;
    } else if let Some(w) = raw {
        media::write(
            resolved.arch(),
            &mut uki_reader,
            uki_size,
            &overlay_files,
            overlay_reader_slice,
            None,
            Some(w),
        )?;
    } else if let Some(w) = uki {
        std::io::copy(&mut uki_reader, w)
            .map_err(|e| WizardError::BuildError(format!("write UKI: {e}")))?;
    } else {
        return Ok(Vec::default());
    }

    overlay_handle
        .await
        .map_err(|e| WizardError::BuildError(format!("join overlay task: {e}")))?
        .map_err(|e| WizardError::BuildError(format!("stream overlay components: {e}")))?;

    sections_handle
        .await
        .map_err(|e| WizardError::BuildError(format!("join UKI build task: {e}")))?
}

pub(crate) async fn write_standalone<W: Write>(
    resolved: &BuildPlan,
    tail_parts: Option<&TailParts>,
    mut kernel: Option<&mut W>,
    mut cmdline: Option<&mut W>,
    mut initramfs: Option<&mut W>,
) -> Result<()> {
    if let Some(ref mut w) = kernel {
        stream_installer_file(resolved, "vmlinuz", w).await?;
    }

    if let Some(ref mut w) = cmdline {
        stream_installer_file(resolved, "cmdline", w).await?;
    }

    if let Some(ref mut w) = initramfs {
        let tail = tail_parts.ok_or_else(|| {
            WizardError::BuildError("initramfs requires extensions but none were pulled".to_owned())
        })?;
        stream_installer_file(resolved, "initramfs.img", w).await?;
        archive::build_tail_from_parts(tail, w)?;
    }

    Ok(())
}

#[expect(clippy::excessive_nesting, reason = "callback closure with match arms")]
async fn stream_overlay_components(
    info: &overlay::Overlay,
    path_to_index: &HashMap<&str, usize>,
    mut writers: Vec<UnixStream>,
) -> Result<()> {
    overlay::pull(info, |path, _size, reader| {
        if let Some(&idx) = path_to_index.get(path) {
            let writer = writers.get_mut(idx).ok_or_else(|| {
                WizardError::BuildError(format!("overlay file index {idx} out of range"))
            })?;
            std::io::copy(reader, writer)
                .map_err(|e| WizardError::BuildError(format!("stream overlay file: {e}")))?;
        }
        Ok(())
    })
    .await
}

async fn stream_installer_file<W: Write>(
    resolved: &BuildPlan,
    target: &'static str,
    writer: &mut W,
) -> Result<()> {
    let installer_ref = resolved.installer().to_owned();
    let arch = resolved.arch();
    installer::pull(&installer_ref, &arch, |path, _size, reader| {
        if path == target {
            std::io::copy(reader, writer).map_err(std::io::Error::other)?;
        }

        Ok(())
    })
    .await
}
