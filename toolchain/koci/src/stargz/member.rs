//! eStargz blob members.

use std::fs::File;

use sha2::Sha256;
use tar::Header;

use super::encode;
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
    members.insert(0, no_prefetch_landmark());

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

    fn entry_type(&self) -> tar::EntryType {
        match *self {
            Self::Dir { .. } => tar::EntryType::Directory,
            Self::Mem { .. } | Self::File { .. } => tar::EntryType::Regular,
        }
    }
}

/// The no-prefetch landmark member written first in every blob.
pub(crate) fn no_prefetch_landmark() -> Member {
    Member::Mem {
        path: NO_PREFETCH_LANDMARK.to_owned(),
        mode: 0o644,
        data: LANDMARK_CONTENT.to_vec(),
    }
}

/// Streams one member and returns its TOC metadata.
pub(crate) fn stream(bytes: &mut Vec<u8>, tar: &mut Sha256, member: &Member) -> Result<Record> {
    let (size, mode) = layout(member)?;
    let name = member.toc_name().to_owned();

    let mut header = Header::new_ustar();
    header.set_mtime(0);
    header.set_mode(mode);
    header.set_size(size);
    header.set_entry_type(member.entry_type());
    header
        .set_path(member.name())
        .map_err(|error| KociError::PushError(format!("set path {name}: {error}")))?;
    header.set_cksum();

    match *member {
        Member::Dir { .. } => encode::emit(bytes, tar, &[], &header, &name, None),
        Member::Mem { ref data, .. } => {
            let mut slice = data.as_slice();

            encode::emit(bytes, tar, &[], &header, &name, Some(&mut slice))
        }
        Member::File { ref file, .. } => {
            let mut reader = file;

            encode::emit(bytes, tar, &[], &header, &name, Some(&mut reader))
        }
    }
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

#[cfg(test)]
mod tests {
    use sha2::Digest as _;

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
