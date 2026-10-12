//! eStargz blob members.

use std::fs::File;
use std::io::{self, Read, Write};

use flate2::Compression;
use flate2::write::GzEncoder;
use oci::digest::sha256_hex;
use sha2::{Digest as _, Sha256};
use tar::Header;

use super::toc::Record;
use super::{Entry, Source, toc as toc_mod};
use crate::error::{KociError, Result};

/// Name of the prefetch landmark recorded by stargz-snapshotter tools.
const PREFETCH_LANDMARK: &str = ".prefetch.landmark";

/// Name of the no-prefetch landmark recorded first in every blob.
const NO_PREFETCH_LANDMARK: &str = ".no.prefetch.landmark";

/// The landmark file's single content byte.
const LANDMARK_CONTENT: [u8; 1] = [0x0f];

/// Returns whether `path` is an eStargz bookkeeping entry, not real content.
#[must_use]
pub(crate) fn is_bookkeeping(path: &str) -> bool {
    path == PREFETCH_LANDMARK || path == NO_PREFETCH_LANDMARK
}

/// Plans the sorted member list: directories, files, and the landmark.
///
/// # Errors
///
/// Returns an error when the entry list is empty or a source cannot be read.
pub(crate) fn plan(entries: &[Entry]) -> Result<Vec<Member>> {
    if entries.is_empty() {
        return Err(KociError::PushError(
            "layer needs at least one entry".to_owned(),
        ));
    }

    let mut files: Vec<&Entry> = entries.iter().collect();
    files.sort_unstable_by(|left, right| left.path.cmp(&right.path));

    let dirs = files
        .iter()
        .flat_map(|entry| parent_dirs(&entry.path))
        .collect::<Vec<_>>();
    let mut members: Vec<Member> = Vec::new();
    for dir in &dirs {
        if let Some(member) = unstaged(dir, &members) {
            members.push(member);
        }
    }

    for entry in &files {
        let staged = stage_member(&entry.source, entry.path.as_str(), entry.mode)
            .map_err(|error| KociError::PushError(error.to_string()))?;
        members.push(staged);
    }
    members.sort_unstable_by_key(|member| member.toc_name().to_owned());
    members.insert(
        0,
        Member::Mem {
            path: NO_PREFETCH_LANDMARK.to_owned(),
            mode: 0o644,
            data: LANDMARK_CONTENT.to_vec(),
        },
    );

    Ok(members)
}

/// One member of the assembled blob: directory, regular file, or bytes.
pub(crate) enum Member {
    Dir {
        path: String,
    },
    Mem {
        path: String,
        mode: u32,
        data: Vec<u8>,
    },
    File {
        path: String,
        mode: u32,
        file: File,
    },
}

impl Member {
    fn name(&self) -> &str {
        match *self {
            Self::Dir { ref path } | Self::Mem { ref path, .. } | Self::File { ref path, .. } => {
                path
            }
        }
    }

    fn toc_name(&self) -> &str {
        self.name().strip_suffix('/').unwrap_or(self.name())
    }
}

/// Streams one member and returns its TOC metadata.
pub(crate) fn stream(bytes: &mut Vec<u8>, tar: &mut Sha256, member: &Member) -> Result<Record> {
    let name = member.toc_name().to_owned();
    let offset = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    let mut payload_digest = Sha256::new();
    let mut encoded = GzEncoder::new(LayerSink { bytes, digest: tar }, Compression::default());

    let (size, mode) = layout(member)?;

    let mut header = Header::new_ustar();
    header.set_mtime(0);
    header.set_mode(mode);
    header.set_size(size);
    if matches!(member, Member::Dir { .. }) {
        header.set_entry_type(tar::EntryType::Directory);
    } else {
        header.set_entry_type(tar::EntryType::Regular);
    }
    header
        .set_path(member.name())
        .map_err(|error| KociError::PushError(format!("set path {name}: {error}")))?;
    header.set_cksum();
    encoded
        .write_all(header.as_bytes())
        .map_err(KociError::IoError)?;

    match *member {
        Member::Dir { .. } => {}
        Member::Mem { ref data, .. } => {
            let mut slice: &[u8] = data;
            stream_payload(&mut encoded, &mut payload_digest, &mut slice)?;
        }
        Member::File { ref file, .. } => {
            let mut reader = file;
            stream_payload(&mut encoded, &mut payload_digest, &mut reader)?;
        }
    }
    pad_member(size, &mut encoded)?;

    let digest = match *member {
        Member::Dir { .. } => None,
        Member::Mem { ref data, .. } => Some(format!("sha256:{}", sha256_hex(data))),
        Member::File { .. } => Some(format!(
            "sha256:{}",
            sha256_hex(&payload_digest.clone().finalize())
        )),
    };
    encoded
        .finish()
        .map_err(|error| KociError::PushError(format!("gzip member: {error}")))?;

    Ok(Record {
        name,
        kind: if matches!(member, Member::Dir { .. }) {
            "dir"
        } else {
            "reg"
        }
        .to_owned(),
        size: if matches!(member, Member::Dir { .. }) {
            None
        } else {
            Some(size)
        },
        mode: Some(u64::from(mode)),
        offset: Some(offset),
        digest: digest.clone(),
        chunk_digest: digest,
    })
}

/// Builds the in-memory TOC member and its JSON digest.
pub(crate) fn toc_member(toc: &[Record]) -> Result<(Member, String)> {
    let (payload, digest) = toc_mod::Toc::encode(toc)?;

    Ok((
        Member::Mem {
            path: toc_mod::TOC_NAME.to_owned(),
            mode: 0o644,
            data: payload,
        },
        digest,
    ))
}

fn layout(member: &Member) -> Result<(u64, u32)> {
    match *member {
        Member::Dir { .. } => Ok((0, 0o755)),
        Member::Mem {
            ref data, ref mode, ..
        } => {
            let size = u64::try_from(data.len()).unwrap_or(u64::MAX);
            Ok((size, *mode))
        }
        Member::File {
            ref file, ref mode, ..
        } => {
            let size = file.metadata().map_err(KociError::IoError)?.len();
            Ok((size, *mode))
        }
    }
}

fn unstaged(dir: &str, members: &[Member]) -> Option<Member> {
    let known = members.iter().any(|member| member.toc_name() == dir);
    (!known).then(|| Member::Dir {
        path: format!("{dir}/"),
    })
}

fn parent_dirs(path: &str) -> Vec<String> {
    let mut dirs = Vec::new();
    let mut rest = path;

    while let Some((parent, _)) = rest.rsplit_once('/') {
        if parent.is_empty() {
            break;
        }
        dirs.push(parent.to_owned());
        rest = parent;
    }

    dirs
}

fn stage_member(source: &Source, name: &str, mode: u32) -> Result<Member> {
    match *source {
        Source::File(ref file) => Ok(Member::File {
            path: name.to_owned(),
            mode,
            file: File::open(file)?,
        }),
        Source::Bytes(ref bytes) => Ok(Member::Mem {
            path: name.to_owned(),
            mode,
            data: bytes.clone(),
        }),
    }
}

fn stream_payload<R: Read + ?Sized>(
    encoded: &mut GzEncoder<LayerSink<'_>>,
    digest: &mut Sha256,
    payload: &mut R,
) -> Result<()> {
    let mut hashed = HashingRead {
        inner: payload,
        digest,
    };
    io::copy(&mut hashed, encoded).map_err(KociError::IoError)?;

    Ok(())
}

fn pad_member(size: u64, encoded: &mut GzEncoder<LayerSink<'_>>) -> Result<()> {
    let aligned = size
        .checked_next_multiple_of(512)
        .ok_or_else(|| KociError::PushError("member padding overflows".to_owned()))?;
    let missing = aligned.saturating_sub(size);
    let missing = usize::try_from(missing).unwrap_or(0);

    let zeros = vec![0_u8; missing];
    io::copy(&mut zeros.as_slice(), encoded).map_err(KociError::IoError)?;

    Ok(())
}

struct LayerSink<'a> {
    bytes: &'a mut Vec<u8>,
    digest: &'a mut Sha256,
}

impl Write for LayerSink<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.digest.update(buf);
        self.bytes.extend_from_slice(buf);

        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct HashingRead<'a, R: Read> {
    inner: R,
    digest: &'a mut Sha256,
}

impl<R: Read> Read for HashingRead<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let copied = self.inner.read(buf)?;
        self.digest.update(buf.get(..copied).unwrap_or(buf));

        Ok(copied)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_includes_files_and_landmark() {
        // ARRANGE
        let dir = tempfile::TempDir::new().expect("temp");
        let source = dir.path().join("catalog.toml");
        std::fs::write(&source, b"api_version = 1\n").expect("write");
        let entries = vec![Entry {
            path: "catalog.toml".to_owned(),
            mode: 0o644,
            source: Source::File(source),
        }];

        // ACT
        let members = plan(&entries).expect("plan");

        // ASSERT
        let names: Vec<&str> = members.iter().map(Member::toc_name).collect();
        assert!(names.contains(&"catalog.toml"));
    }

    #[test]
    fn stream_writes_file_member_into_blob() {
        // ARRANGE
        let dir = tempfile::TempDir::new().expect("temp");
        let source = dir.path().join("catalog.toml");
        std::fs::write(&source, b"api_version = 1\n").expect("write");
        let member = Member::File {
            path: "catalog.toml".to_owned(),
            mode: 0o644,
            file: std::fs::File::open(&source).expect("open"),
        };
        let mut bytes = Vec::new();
        let mut tar_digest = Sha256::new();

        // ACT
        let entry = stream(&mut bytes, &mut tar_digest, &member).expect("stream");
        let names: Vec<String> =
            tar::Archive::new(flate2::read::MultiGzDecoder::new(bytes.as_slice()))
                .entries()
                .expect("entries")
                .filter_map(std::result::Result::ok)
                .map(|entry| String::from_utf8_lossy(entry.path_bytes().as_ref()).into_owned())
                .collect();

        // ASSERT
        assert_eq!(names, vec!["catalog.toml"]);
        assert_eq!(entry.name, "catalog.toml");
    }
}
