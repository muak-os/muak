//! Converting published images into eStargz layers in place.

use alloc::collections::BTreeMap;
use std::path::PathBuf;

use bytes::Bytes;
use oci::arch::Arch;
use oci_client::auth::Access;
use oci_client::client::Client;
use oci_client::manifest;
use serde_json::Value;

use crate::error::{KociError, Result};
use crate::progress::Noop;
use crate::pull;
use crate::push;
use crate::registry;
use crate::runtime;
use crate::stargz::toc::TOC_NAME;

/// One platform line of an image: its manifest digest and architecture.
struct Platform {
    /// `sha256:...` manifest digest for index bookkeeping.
    digest: String,
    /// Architecture this platform targets.
    arch: Arch,
}

/// One repacked platform: manifest bytes, their digest, and the line replaced.
struct Converted {
    /// Platform manifest bytes.
    bytes: Vec<u8>,
    /// `sha256:...` digest over the manifest bytes.
    digest: String,
    /// Digest of the manifest line this conversion replaces.
    source: String,
}

/// Single-platform images carry exactly this many lines.
const SINGLE: usize = 1;

/// Rewrite every platform manifest under `reference` with one eStargz layer.
///
/// Each platform's live entries (whiteouts already applied by the walk) are
/// staged into a disk directory and repacked sorted, so the converted image
/// streams like the ones `koci push` builds. Configs rebuild from the
/// platform architecture, as `koci push` does.
///
/// # Errors
///
/// Returns an error when the image cannot be resolved, entries cannot be
/// staged, or any registry operation fails.
pub fn run(reference: &str) -> Result<()> {
    runtime::runtime()?.block_on(convert(reference))
}

async fn convert(reference: &str) -> Result<()> {
    let client = registry::connect(reference, Access::PullPush).await?;
    let tag = client.image().manifest_ref.clone();
    let root_json = manifest::fetch(&client, &tag).await?;
    let root_media = media_type(&root_json);
    let platforms = platform_list(&client, &root_json).await?;
    let is_index = platforms.len() != SINGLE;

    let mut converted: Vec<Converted> = Vec::with_capacity(platforms.len());
    for platform in platforms {
        converted.push(pack_platform(reference, &platform).await?);
    }
    let mut index: Option<Value> = is_index
        .then(|| {
            serde_json::from_str(&root_json)
                .map_err(|error| KociError::CopyError(format!("parse index: {error}")))
        })
        .transpose()?;

    for product in &converted {
        publish_platform(&client, is_index, product).await?;
        record_index_line(&mut index, is_index, product)?;
    }

    if let Some(index) = index {
        let media = root_media.to_owned();
        let bytes = serde_json::to_vec(&index)
            .map_err(|error| KociError::CopyError(format!("serialize index: {error}")))?;
        manifest::put(
            &client,
            &client.image().manifest_ref,
            &media,
            Bytes::from(bytes),
        )
        .await?;
    } else {
        let product = converted
            .first()
            .ok_or_else(|| KociError::CopyError("conversion produced no manifest".to_owned()))?;
        let bytes = Bytes::from(product.bytes.clone());
        manifest::put(&client, &client.image().manifest_ref, root_media, bytes).await?;
    }

    Ok(())
}

/// Publishes one converted platform under its digest for an index.
async fn publish_platform(client: &Client, is_index: bool, product: &Converted) -> Result<()> {
    if !is_index {
        return Ok(());
    }

    let bytes = Bytes::from(product.bytes.clone());
    manifest::put(client, &product.digest, MANIFEST_MEDIA, bytes).await?;

    Ok(())
}

/// Records the converted digest under its index line.
///
/// # Errors
///
/// Returns an error when the index has no matching line.
fn record_index_line(index: &mut Option<Value>, is_index: bool, product: &Converted) -> Result<()> {
    let Some(index) = index.as_mut().filter(|_| is_index) else {
        return Ok(());
    };

    write_index_line(index, product)
}

/// Media type of a converted single platform manifest.
const MANIFEST_MEDIA: &str = "application/vnd.oci.image.manifest.v1+json";

/// Builds the platform list: one per index entry, or a single-arch line.
async fn platform_list(client: &Client, root_json: &str) -> Result<Vec<Platform>> {
    let root = oci::manifest::parse(root_json)?;
    if !root.manifests.is_empty() {
        return root.manifests.iter().map(index_platform).collect();
    }

    let arch = single_arch(client, root_json).await?;

    Ok(vec![Platform {
        digest: client.image().manifest_ref.clone(),
        arch,
    }])
}

/// Reads the architecture of a single-arch manifest from its config blob.
async fn single_arch(client: &Client, root_json: &str) -> Result<Arch> {
    let root = oci::manifest::parse(root_json)?;
    let digest = root
        .config
        .as_ref()
        .map(|config| config.digest.as_str())
        .ok_or_else(|| KociError::CopyError("manifest without config digest".to_owned()))?;
    let config = download_blob(client, digest).await?;
    let json = core::str::from_utf8(&config)
        .map_err(|error| KociError::CopyError(format!("config is not UTF-8: {error}")))?;
    let platform = oci::manifest::config_platform(json)?;
    let architecture = platform
        .architecture
        .ok_or_else(|| KociError::CopyError("config without architecture".to_owned()))?;

    architecture.parse::<Arch>().map_err(KociError::CopyError)
}

/// Downloads one blob into memory, verifying its digest.
async fn download_blob(client: &Client, digest: &str) -> Result<Vec<u8>> {
    let http = client.http();
    let image = client.image();
    let authorization = client.authorization().map(str::to_owned);

    pull::download::blob(http, image, digest, authorization.as_deref()).await
}

/// One index line to a target platform.
fn index_platform(descriptor: &oci::model::Descriptor) -> Result<Platform> {
    let platform = descriptor
        .platform
        .as_ref()
        .and_then(arch_of)
        .ok_or_else(|| KociError::CopyError("index manifest without platform".to_owned()))?;

    Ok(Platform {
        digest: descriptor.digest.clone(),
        arch: platform,
    })
}

/// The architecture an index platform descriptor targets.
fn arch_of(platform: &oci::model::Platform) -> Option<Arch> {
    platform.architecture.as_deref()?.parse::<Arch>().ok()
}

/// Repacks one platform's live entries into a fresh eStargz image manifest.
async fn pack_platform(reference: &str, platform: &Platform) -> Result<Converted> {
    let session = pull::session::open_async(reference, &platform.arch, None).await?;
    let content = session.content();
    let mut stage = Stage::new()?;
    {
        let stage_ref = &mut stage;
        pull::layer::walk(
            session.client(),
            session.cache(),
            content,
            session.layers(),
            &Noop,
            |_layer_idx, entry, info| stage_entry(stage_ref, entry, info),
        )
        .await?;
    }

    let entries = stage.into_entries()?;
    let (bytes, digest) = push::pack(session.client(), platform.arch, &entries).await?;

    Ok(Converted {
        bytes,
        digest,
        source: platform.digest.clone(),
    })
}

/// eStargz bookkeeping entries that must not appear twice in the output.
const STARGZ_BOOKKEEPING: [&str; 2] = [".prefetch.landmark", ".no.prefetch.landmark"];

fn is_stargz_metadata(path: &str) -> bool {
    path == TOC_NAME || path == STARGZ_BOOKKEEPING[0] || path == STARGZ_BOOKKEEPING[1]
}

/// Stages one regular file entry, skipping directories.
fn stage_entry(
    stage: &mut Stage,
    mut entry: tar::Entry<&mut pull::download::LayerReader>,
    info: pull::scan::EntryInfo<'_>,
) -> Result<()> {
    match info {
        pull::scan::EntryInfo::File(path, _size, _) => {
            if is_stargz_metadata(path.as_os_str().to_str().unwrap_or_default()) {
                let mut sink = std::io::sink();
                std::io::copy(&mut entry, &mut sink).map_err(KociError::IoError)?;
                return Ok(());
            }
            stage.entry(&path.to_string_lossy(), &mut entry)?;
        }
        pull::scan::EntryInfo::Skip | pull::scan::EntryInfo::Whiteout(_) => {
            let mut sink = std::io::sink();
            std::io::copy(&mut entry, &mut sink).map_err(KociError::IoError)?;
        }
    }

    Ok(())
}

/// Writes one converted digest and size into the matching index line.
///
/// # Errors
///
/// Returns an error when the index has no manifests or the converted manifest
/// does not match any line.
fn write_index_line(index: &mut Value, product: &Converted) -> Result<()> {
    let lines = index
        .get_mut("manifests")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| KociError::CopyError("index without manifests".to_owned()))?;

    for line in lines {
        if line.get("digest").and_then(Value::as_str) == Some(product.source.as_str()) {
            line["digest"] = Value::String(product.digest.clone());
            line["size"] = Value::from(product.bytes.len());

            return Ok(());
        }
    }

    Err(KociError::CopyError(
        "converted manifest not found in the index".to_owned(),
    ))
}

/// The media type carried by a fetched root manifest.
fn media_type(root_json: &str) -> &'static str {
    match serde_json::from_str::<Value>(root_json) {
        Ok(root) => match root.get("mediaType").and_then(Value::as_str) {
            Some("application/vnd.oci.image.index.v1+json") => {
                "application/vnd.oci.image.index.v1+json"
            }
            Some("application/vnd.docker.distribution.manifest.v2+json") => {
                "application/vnd.docker.distribution.manifest.v2+json"
            }
            _ => "application/vnd.oci.image.manifest.v1+json",
        },
        Err(_) => "application/vnd.oci.image.manifest.v1+json",
    }
}

/// Stages union walk output on disk, last write winning per path.
struct Stage {
    root: PathBuf,
    files: BTreeMap<String, push::Entry>,
}

impl Stage {
    /// Creates the staging root under the shared temporary directory.
    fn new() -> Result<Self> {
        let root = std::env::temp_dir().join(format!(
            "koci-convert-{}-{}",
            std::process::id(),
            SystemTimestamp::now(),
        ));
        std::fs::create_dir_all(&root).map_err(KociError::IoError)?;
        let files = BTreeMap::new();

        Ok(Self { root, files })
    }

    /// Streams one walk entry into the staging tree, skipping directories.
    fn entry(&mut self, path: &str, reader: &mut dyn std::io::Read) -> Result<()> {
        if path.ends_with('/') {
            return Ok(());
        }

        let sequence = self.files.len();
        let source = self.root.join(sequence.to_string());
        {
            let mut file = std::fs::File::create(&source).map_err(KociError::IoError)?;
            std::io::copy(reader, &mut file).map_err(KociError::IoError)?;
        }
        self.files.insert(
            path.to_owned(),
            push::Entry {
                path: path.to_owned(),
                source,
            },
        );

        Ok(())
    }

    /// Freezes the staged entries into a validated push inventory.
    fn into_entries(self) -> Result<Vec<push::Entry>> {
        let entries: Vec<push::Entry> = self.files.into_values().collect();
        push::validate(&entries)?;

        Ok(entries)
    }
}

/// A nanosecond tick for staging directory names.
struct SystemTimestamp;

impl SystemTimestamp {
    /// Current Unix nanos, zero on clock failure.
    #[must_use]
    fn now() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |span| span.as_nanos())
    }
}
