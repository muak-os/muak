//! Rebuilds an eStargz layer from a plain tar stream, preserving entry order.

use std::io::Read;

use sha2::{Digest as _, Sha256};
use tar::Archive;

use super::toc::Record;
use super::{LayerBlob, assemble, encode, gnu, member};
use crate::error::{KociError, Result};

/// Rebuilds `reader`, an uncompressed tar stream, into an eStargz layer.
///
/// # Errors
///
/// Returns an error when the tar stream cannot be read or a member cannot be
/// encoded.
pub fn from_tar<R: Read>(reader: R) -> Result<LayerBlob> {
    let mut bytes = Vec::new();
    let mut tar_digest = Sha256::new();
    let mut toc = Vec::new();

    toc.push(member::stream(
        &mut bytes,
        &mut tar_digest,
        &member::no_prefetch_landmark(),
    )?);

    let mut archive = Archive::new(reader);
    for entry in archive.entries().map_err(KociError::IoError)? {
        let mut entry = entry.map_err(KociError::IoError)?;
        if let Some(record) = repack(&mut bytes, &mut tar_digest, &mut entry)? {
            toc.push(record);
        }
    }

    assemble(bytes, &mut tar_digest, &toc)
}

fn repack<R: Read>(
    bytes: &mut Vec<u8>,
    tar: &mut Sha256,
    entry: &mut tar::Entry<'_, R>,
) -> Result<Option<Record>> {
    let source_type = entry.header().entry_type();
    let path = entry.path_bytes().to_vec();
    if member::is_bookkeeping(&String::from_utf8_lossy(&path)) {
        return Ok(None);
    }

    let mut header = entry.header().clone();
    header.set_mtime(0);
    let regular = source_type.is_file() || source_type.is_contiguous();
    if regular {
        header.set_size(entry.size());
    } else {
        header.set_size(0);
    }

    let mut prefix = gnu::long_name(&mut header, &path)?;
    if (source_type.is_symlink() || source_type.is_hard_link())
        && let Some(link) = entry.link_name_bytes()
    {
        prefix.extend(gnu::long_link(&mut header, link.as_ref())?);
    }
    header.set_cksum();

    let name = String::from_utf8_lossy(&path)
        .trim_end_matches('/')
        .to_owned();
    let record = if regular {
        let mut reader: &mut dyn Read = entry;

        encode::emit(bytes, tar, &prefix, &header, &name, Some(&mut reader))?
    } else {
        encode::emit(bytes, tar, &prefix, &header, &name, None)?
    };

    Ok(Some(record))
}
