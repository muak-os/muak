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
use crate::artifact::Artifact;
use crate::error::{Result, WizardError};
use crate::resolve::BuildPlan;
use crate::source::{installer, overlay};

struct OverlayPipes {
    files: Vec<FileMeta<'static>>,
    readers: Vec<UnixStream>,
    handle: JoinHandle<Result<()>>,
}

pub(crate) struct BuildPostConfig<'a> {
    pub resolved: &'a BuildPlan,
    pub installer_meta: &'a installer::Metadata,
    pub tail_parts: Option<&'a TailParts>,
    pub tail_size: u64,
    pub signing_key: Option<&'a SigningPair<'a>>,
}

pub(crate) async fn build(
    config: BuildPostConfig<'_>,
    targets: Vec<(Artifact, &mut dyn Write)>,
) -> Result<Vec<Section>> {
    let mut uki_w = None;
    let mut iso_w = None;
    let mut raw_w = None;
    let mut kernel_w: Option<&mut dyn Write> = None;
    let mut cmdline_w: Option<&mut dyn Write> = None;
    let mut initramfs_w: Option<&mut dyn Write> = None;
    for (kind, writer) in targets {
        match kind {
            Artifact::Uki => uki_w = Some(writer),
            Artifact::Iso => iso_w = Some(writer),
            Artifact::Raw => raw_w = Some(writer),
            Artifact::Kernel => kernel_w = Some(writer),
            Artifact::Cmdline => cmdline_w = Some(writer),
            Artifact::Initramfs => initramfs_w = Some(writer),
        }
    }

    let needs_uki = uki_w.is_some() || iso_w.is_some() || raw_w.is_some();
    let needs_media = iso_w.is_some() || raw_w.is_some();

    let mut overlay_pipes = if needs_media {
        match config.resolved.overlay() {
            Some(info) => Some(setup_overlay_pipes(info).await?),
            None => None,
        }
    } else {
        None
    };

    let mut uki_build = if needs_uki {
        Some(uki::build(
            config.installer_meta,
            config.tail_size,
            config.signing_key,
        )?)
    } else {
        None
    };

    if let Some(ref mut build) = uki_build {
        uki::write_tail(
            build,
            config.tail_parts.ok_or_else(|| {
                WizardError::BuildError("tail_parts required for UKI build".to_owned())
            })?,
        )?;
    }

    if let Some(ref mut build) = uki_build {
        let uki_stub_w = Some(&mut build.stub_w);
        let uki_data_w = Some(&mut build.data_w);
        pull_and_tee(
            config.resolved,
            uki_stub_w,
            uki_data_w,
            kernel_w,
            cmdline_w,
            initramfs_w,
            config.tail_parts,
        )
        .await?;
    } else {
        pull_and_tee(
            config.resolved,
            None,
            None,
            kernel_w,
            cmdline_w,
            initramfs_w,
            config.tail_parts,
        )
        .await?;
    }

    let sections =
        write_media_artifacts(uki_build, uki_w, iso_w, raw_w, &mut overlay_pipes, &config).await?;

    if let Some(pipes) = overlay_pipes {
        pipes
            .handle
            .await
            .map_err(|e| WizardError::BuildError(format!("join overlay task: {e}")))?
            .map_err(|e| WizardError::BuildError(format!("stream overlay components: {e}")))?;
    }

    Ok(sections)
}

async fn write_media_artifacts(
    uki_build: Option<uki::Build>,
    uki_w: Option<&mut dyn Write>,
    iso_w: Option<&mut dyn Write>,
    raw_w: Option<&mut dyn Write>,
    overlay_pipes: &mut Option<OverlayPipes>,
    config: &BuildPostConfig<'_>,
) -> Result<Vec<Section>> {
    let Some(uki_build) = uki_build else {
        return Ok(Vec::default());
    };

    let (mut uki_reader, uki_size, sections_handle) = uki::collect(uki_build);

    let empty_files = Vec::<FileMeta<'_>>::new();
    let (overlay_files, mut overlay_reader_refs) = if let Some(pipes) = overlay_pipes.as_mut() {
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

    if let Some(w) = uki_w {
        media::write(
            config.resolved.arch(),
            &mut uki_reader,
            uki_size,
            overlay_files,
            overlay_reader_slice,
            Some(w),
            None,
        )?;
    } else if let Some(w) = raw_w {
        media::write(
            config.resolved.arch(),
            &mut uki_reader,
            uki_size,
            overlay_files,
            overlay_reader_slice,
            None,
            Some(w),
        )?;
    } else if let Some(w) = iso_w {
        media::write(
            config.resolved.arch(),
            &mut uki_reader,
            uki_size,
            overlay_files,
            overlay_reader_slice,
            Some(w),
            None,
        )?;
    } else {
        media::write(
            config.resolved.arch(),
            &mut uki_reader,
            uki_size,
            overlay_files,
            overlay_reader_slice,
            None,
            None,
        )?;
    }

    sections_handle
        .await
        .map_err(|e| WizardError::BuildError(format!("join UKI build task: {e}")))?
}

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

struct TeeTargets<'a> {
    uki_stub_w: Option<&'a mut UnixStream>,
    uki_data_w: Option<&'a mut UnixStream>,
    kernel: Option<&'a mut dyn Write>,
    cmdline: Option<&'a mut dyn Write>,
    initramfs: Option<&'a mut dyn Write>,
    tail_parts: Option<&'a TailParts>,
}

impl TeeTargets<'_> {
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

fn copy_if_some(
    reader: &mut dyn Read,
    writer: &mut Option<&mut (impl Write + ?Sized)>,
) -> std::io::Result<()> {
    if let Some(w) = writer.as_deref_mut() {
        std::io::copy(reader, w)?;
    }

    Ok(())
}

async fn pull_and_tee<'a>(
    resolved: &BuildPlan,
    uki_stub_w: Option<&'a mut UnixStream>,
    uki_data_w: Option<&'a mut UnixStream>,
    kernel: Option<&'a mut dyn Write>,
    cmdline: Option<&'a mut dyn Write>,
    initramfs: Option<&'a mut dyn Write>,
    tail_parts: Option<&'a TailParts>,
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
