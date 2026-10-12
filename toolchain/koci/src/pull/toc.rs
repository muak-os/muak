//! TOC reads: eStargz entry tables read from cached blobs and HTTP ranges.

use std::io::{self, Read};

use flate2::read::GzDecoder;
use oci::arch::Arch;
use oci::model::Descriptor;
use oci_client::auth::Access;
use oci_client::client::Client;
use tar::Archive;

use crate::error::{KociError, Result};
use crate::pull::fetch;
use crate::pull::{cache, content};
use crate::registry;
use crate::runtime;
use crate::signature::Verification;
use crate::stargz::toc::{FOOTER_SIZE, Record, TOC_NAME, Toc, parse_footer};

/// The `(path, size)` of one regular-file layer entry, measured without decoding the layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryInfo {
    /// Full tar path of the entry.
    pub path: String,
    /// Uncompressed byte size of the entry payload.
    pub size: u64,
}

/// Measure the regular-file entries of the platform manifest matching `arch`.
///
/// # Errors
///
/// Returns an error if the manifest cannot be resolved, signature
/// verification fails, a footer or TOC is missing, or a fetch fails.
pub fn entries(
    reference: &str,
    arch: &Arch,
    verification: Option<&Verification<'_>>,
) -> Result<Vec<EntryInfo>> {
    runtime::runtime()?.block_on(measure_all(reference, arch, verification))
}

async fn measure_all(
    reference: &str,
    arch: &Arch,
    verification: Option<&Verification<'_>>,
) -> Result<Vec<EntryInfo>> {
    let client = registry::connect(reference, Access::Pull).await?;
    let cache = cache::Store::new();
    let content = content::Content::new();
    let json = super::resolve::platform_manifest_json(&client, &cache, arch, verification).await?;
    let parsed = oci::manifest::parse(&json)?;

    let mut measured: Vec<EntryInfo> = Vec::new();
    for layer in &parsed.layers {
        let entries = layer_entries(&client, &cache, &content, layer).await?;
        let files = entries
            .into_iter()
            .filter(|entry| entry.kind == "reg")
            .filter_map(|entry| {
                let size = entry.size?;
                Some(EntryInfo {
                    path: entry.name,
                    size,
                })
            });
        measured.extend(files);
    }

    Ok(measured)
}

async fn layer_entries(
    client: &Client,
    cache: &cache::Store,
    content: &content::Content,
    layer: &Descriptor,
) -> Result<Vec<Record>> {
    let size = layer.size;
    let authorization = client.authorization().map(str::to_owned);
    let fetched = fetch::fetch_one(
        cache,
        content,
        client.http(),
        client.image(),
        &layer.digest,
        authorization.as_deref(),
    )
    .await?;

    let decoded = match fetched {
        Some(bytes) => decode_from_bytes(&bytes, size),
        None => decode_from_disk(content, &layer.digest, size),
    }?;

    Ok(decoded)
}

fn decode_from_bytes(blob: &[u8], size: u64) -> Result<Vec<Record>> {
    let footer_start =
        usize::try_from(size.saturating_sub(u64::try_from(FOOTER_SIZE).unwrap_or(0))).unwrap_or(0);
    let tail = slice(blob, footer_start, blob.len())?;
    let offset = parse_footer_offset(tail)?;
    let toc_end = footer_start;
    let start = usize::try_from(offset).unwrap_or(0);
    let member = slice(blob, start, toc_end)?;

    parse_toc_member(member)
}

fn decode_from_disk(content: &content::Content, digest: &str, size: u64) -> Result<Vec<Record>> {
    let footer_start = size.saturating_sub(u64::try_from(FOOTER_SIZE).unwrap_or(0));

    let mut file = content.open_blob(digest).map_err(KociError::IoError)?;
    let mut footer = [0_u8; FOOTER_SIZE];
    skip(&mut file, footer_start)?;
    file.read_exact(&mut footer).map_err(KociError::IoError)?;
    let offset = parse_footer_offset(&footer)?;

    let file = content.open_blob(digest).map_err(KociError::IoError)?;
    let span = footer_start.saturating_sub(offset);
    skip(&file, offset)?;
    let toc = fill_span(&file, span)?;
    parse_toc_member(&toc)
}

fn skip(mut reader: impl Read, count: u64) -> Result<()> {
    io::copy(&mut reader.by_ref().take(count), &mut io::sink()).map_err(KociError::IoError)?;

    Ok(())
}

fn fill_span(mut reader: impl Read, span: u64) -> Result<Vec<u8>> {
    let mut bytes = vec![0_u8; usize::try_from(span).unwrap_or(0)];
    Read::read_exact(&mut reader, &mut bytes).map_err(KociError::IoError)?;

    Ok(bytes)
}

fn slice(body: &[u8], from: usize, to: usize) -> Result<&[u8]> {
    let to = to.max(from).min(body.len());

    body.get(from..to)
        .ok_or_else(|| KociError::Pull("range fetch fell outside the layer blob".to_owned()))
}

fn parse_footer_offset(tail: &[u8]) -> Result<u64> {
    parse_footer(tail).ok_or_else(|| {
        KociError::Pull("carries no eStargz footer; publish it with koci push".to_owned())
    })
}

fn parse_toc_member(body: &[u8]) -> Result<Vec<Record>> {
    let stream = GzDecoder::new(body);
    for entry in Archive::new(stream).entries()? {
        let mut entry = entry.map_err(KociError::IoError)?;
        if entry.path_bytes().as_ref() == TOC_NAME.as_bytes() {
            let mut payload = Vec::new();
            entry
                .read_to_end(&mut payload)
                .map_err(KociError::IoError)?;

            return decode_toc_payload(&payload);
        }
    }

    Err(KociError::Pull(format!(
        "TOC member {TOC_NAME} missing from its tar member",
    )))
}

fn decode_toc_payload(payload: &[u8]) -> Result<Vec<Record>> {
    let toc: Toc = serde_json::from_slice(payload)
        .map_err(|error| KociError::Pull(format!("decode {TOC_NAME}: {error}")))?;

    Ok(toc.into_entries())
}
