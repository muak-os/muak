//! Build pipeline orchestration.

use std::io::{Cursor, Read as _, Write};
use std::os::unix::net::UnixStream;

use ring::digest;
use sbolt::keys::SigningPair;
use sbolt::signature;
use tokio::task::spawn_blocking;
use yuki::BuildInput;
use yuki::SizedPart;
use yuki::section::Section;

use super::archive;
use super::media;
use super::stage::{self, InstallerAssets};
use crate::artifact::Artifact;
use crate::error::{Result, WizardError};
use crate::resolve::ResolvedProfile;

/// Prebuilt intermediate artifacts shared across rendering paths.
pub(crate) struct Prepared {
    pub assets: InstallerAssets,
    pub initramfs_tail: Vec<u8>,
    pub sections: Vec<Section>,
    pub section_hashes: Vec<[u8; 32]>,
}

/// Pulls the installer, builds the initramfs tail, and builds a UKI.
pub(crate) async fn prepare(
    resolved_profile: &ResolvedProfile,
    profile_bytes: &[u8],
    signing_key: Option<&SigningPair<'_>>,
    uki_writer: &mut impl Write,
) -> Result<Prepared> {
    let installer = stage::pull_installer(resolved_profile, None).await?;
    let assets = stage::load_installer_assets(&installer)?;
    let initramfs_tail = archive::build_initramfs_tail(resolved_profile, profile_bytes).await?;

    let stub = stage::read_file(&assets.stub, "stub")?;
    let kernel = stage::read_file(&assets.kernel, "kernel")?;
    let cmdline = stage::read_file(&assets.cmdline, "cmdline")?;

    let cmdline_hash = sha256(&[&cmdline]);
    let kernel_hash = sha256(&[&kernel]);

    let base_file = assets.initramfs.clone();
    let base_len = base_file.len;
    let tail = initramfs_tail.clone();
    let initramfs_len = base_len.saturating_add(u64::try_from(tail.len()).unwrap_or(u64::MAX));
    let stub_len = u64::try_from(stub.len()).unwrap_or(u64::MAX);
    let kernel_len = u64::try_from(kernel.len()).unwrap_or(u64::MAX);
    let cmdline_len = u64::try_from(cmdline.len()).unwrap_or(u64::MAX);

    let (writer, mut reader) =
        UnixStream::pair().map_err(|e| WizardError::BuildError(format!("create pipe: {e}")))?;

    let sections_handle = spawn_blocking(move || {
        let mut stub_reader = Cursor::new(stub);
        let mut kernel_reader = Cursor::new(kernel);
        let mut cmdline_reader = Cursor::new(cmdline);
        let base_reader = base_file
            .open()
            .map_err(|e| WizardError::BuildError(format!("open initramfs: {e}")))?;
        let tail_reader = Cursor::new(tail.as_slice());
        let mut initramfs_reader = base_reader.chain(tail_reader);

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
        yuki::build(input, &mut &writer)
            .map_err(|e| WizardError::BuildError(format!("build UKI: {e}")))
    });

    if let Some(key) = signing_key {
        signature::sign(&mut reader, key.signer, key.certificate, uki_writer)
            .map_err(|e| WizardError::BuildError(format!("sign UKI: {e}")))?;
    } else {
        std::io::copy(&mut reader, uki_writer)
            .map_err(|e| WizardError::BuildError(format!("read UKI pipe: {e}")))?;
    }

    let sections = sections_handle
        .await
        .map_err(|e| WizardError::BuildError(format!("join UKI build task: {e}")))??;

    let initrd_hash = sha256(&[&assets.initramfs.data, &initramfs_tail]);

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

    Ok(Prepared {
        assets,
        initramfs_tail,
        sections,
        section_hashes,
    })
}

/// Metadata returned alongside written artifacts.
pub(crate) struct Metadata {
    pub sections: Vec<Section>,
    pub section_hashes: Vec<[u8; 32]>,
    pub overlay_files: Vec<esp::EspFile>,
}

/// Builds the requested artifacts sharing a single resolution.
///
/// # Errors
///
/// Returns an error when pulling, staging, building, signing, or writing fails.
pub async fn artifacts<W: Write>(
    resolved_profile: &ResolvedProfile,
    requested: &[Artifact],
    signing_key: Option<&SigningPair<'_>>,
    profile_bytes: &[u8],
    writers: super::ArtifactWriters<'_, W>,
) -> Result<Metadata> {
    let super::ArtifactWriters {
        uki,
        kernel,
        cmdline,
        initramfs,
        iso,
        raw,
    } = writers;

    let prepared = if requested.contains(&Artifact::Iso) || requested.contains(&Artifact::Raw) {
        // TODO: Buffer UKI in memory for EspSpec (ISO/Raw) — deferred optimization.
        let mut uki_buf = Vec::new();
        let prepared = prepare(resolved_profile, profile_bytes, signing_key, &mut uki_buf).await?;
        if let Some(w) = uki {
            w.write_all(&uki_buf)
                .map_err(|e| WizardError::BuildError(format!("write UKI: {e}")))?;
        }
        if let Some(w) = iso {
            media::iso_to_writer(resolved_profile, &uki_buf, w).await?;
        }
        if let Some(w) = raw {
            let overlay = pull_overlay_if_present(resolved_profile).await?;
            media::raw_to_writer(resolved_profile, &overlay, &uki_buf, w).await?;
        }
        prepared
    } else if let Some(w) = uki {
        prepare(resolved_profile, profile_bytes, signing_key, w).await?
    } else {
        prepare(
            resolved_profile,
            profile_bytes,
            signing_key,
            &mut std::io::sink(),
        )
        .await?
    };

    if let Some(w) = kernel {
        let data = stage::read_file(&prepared.assets.kernel, "kernel")?;
        w.write_all(&data)
            .map_err(|e| WizardError::BuildError(format!("write kernel: {e}")))?;
    }
    if let Some(w) = cmdline {
        let data = stage::read_file(&prepared.assets.cmdline, "cmdline")?;
        w.write_all(&data)
            .map_err(|e| WizardError::BuildError(format!("write cmdline: {e}")))?;
    }
    if let Some(w) = initramfs {
        archive::write_initramfs_to_writer(&prepared.assets, &prepared.initramfs_tail, w)?;
    }

    let overlay_files = pull_overlay_if_present(resolved_profile).await?;

    Ok(Metadata {
        sections: prepared.sections,
        section_hashes: prepared.section_hashes,
        overlay_files,
    })
}

pub(crate) async fn pull_overlay_if_present(
    resolved_profile: &ResolvedProfile,
) -> Result<Vec<esp::EspFile>> {
    if let Some(overlay) = resolved_profile.overlay() {
        stage::pull_overlay(overlay, &resolved_profile.arch(), None)
            .await
            .map_err(|e| WizardError::BuildError(format!("pull overlay: {e}")))
    } else {
        Ok(vec![])
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
