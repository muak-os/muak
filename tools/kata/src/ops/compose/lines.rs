//! Release line discovery and ordering.

use alloc::collections::BTreeSet;
use std::cmp::Ordering;
use std::path::Path;

use crate::error::Result;
use crate::repository;
use crate::schema::documents::{CoreDocument, Document};
use crate::schema::entries::NamedEntry;
use crate::schema::kinds::Kind;

/// Lines present in a docs root, oldest first, unparsable lines are ignored.
pub(crate) fn scan(root: &Path) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let Ok(entries) = std::fs::read_dir(root.join(Kind::Core.dir())) else {
        return lines;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "toml") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if parse_version(stem).is_some() {
            lines.push(stem.to_owned());
        }
    }

    lines.sort_by(|left, right| compare(left, right));

    lines
}

/// The newest line of a docs root.
pub(crate) fn newest(root: &Path) -> Option<String> {
    scan(root).pop()
}

/// Compare two line names, pre-releases sort below their release.
#[must_use]
pub(crate) fn compare(left: &str, right: &str) -> Ordering {
    let Some((left_major, left_minor, left_patch, left_pre)) = parse_version(left) else {
        return Ordering::Equal;
    };
    let Some((right_major, right_minor, right_patch, right_pre)) = parse_version(right) else {
        return Ordering::Equal;
    };

    left_major
        .cmp(&right_major)
        .then(left_minor.cmp(&right_minor))
        .then(left_patch.cmp(&right_patch))
        .then_with(|| match (left_pre, right_pre) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Greater,
            (Some(_), None) => Ordering::Less,
            (Some(left_pre), Some(right_pre)) => left_pre.cmp(&right_pre),
        })
}

/// Entry identities (`kind/identity`) of every document of a line.
///
/// # Errors
///
/// Returns an error when a document exists but cannot be loaded.
pub(crate) fn identities(root: &Path, line: &str) -> Result<BTreeSet<String>> {
    let mut identities = BTreeSet::new();

    for kind in Kind::all() {
        if !repository::document_path(kind, root, line).exists() {
            continue;
        }
        let document = repository::load(kind, root, line)?;
        match document {
            Document::Core(core) => insert_core(&mut identities, &core),
            Document::Overlays(overlays) => {
                insert_named(&mut identities, "overlays", &overlays.overlays);
            }
            Document::Extensions(extensions) => {
                insert_named(&mut identities, "extensions", &extensions.extensions);
            }
        }
    }

    Ok(identities)
}

fn insert_core(identities: &mut BTreeSet<String>, core: &CoreDocument) {
    for kernel in &core.kernels {
        identities.insert(format!("kernels/{}", kernel.source));
    }
    if core.stub.is_some() {
        identities.insert("stub".to_owned());
    }
    if core.installer.is_some() {
        identities.insert("installer".to_owned());
    }
}

fn insert_named(identities: &mut BTreeSet<String>, prefix: &str, entries: &[NamedEntry]) {
    for entry in entries {
        identities.insert(format!("{prefix}/{}", entry.name));
    }
}

fn parse_version(line: &str) -> Option<(u64, u64, u64, Option<String>)> {
    let rest = line.strip_prefix('v')?;
    let (base, pre_release) = match rest.split_once('-') {
        Some((base, pre)) => (base, Some(pre.to_owned())),
        None => (rest, None),
    };

    let mut numbers = base.split('.');
    let major = numbers.next()?.parse().ok()?;
    let minor = numbers.next()?.parse().ok()?;
    let patch = numbers.next()?.parse().ok()?;
    if numbers.next().is_some() {
        return None;
    }

    Some((major, minor, patch, pre_release))
}

#[cfg(test)]
mod tests {
    use std::cmp::Ordering;
    use std::fs;

    use tempfile::TempDir;

    use super::{compare, newest, parse_version, scan};

    #[test]
    fn parse_version_reads_base_and_pre_release() {
        // ARRANGE / ACT
        let parsed = parse_version("v1.2.3-beta");

        // ASSERT
        assert_eq!(parsed, Some((1, 2, 3, Some("beta".to_owned()))),);
    }

    #[test]
    fn parse_version_rejects_non_lines() {
        // ACT / ASSERT
        assert!(parse_version("1.2").is_none());
        assert!(parse_version("notes").is_none());
    }

    #[test]
    fn compare_orders_pre_releases_below_releases() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(compare("v1.0.0-beta", "v1.0.0"), Ordering::Less);
        assert_eq!(compare("v1.0.0", "v1.1.0-beta"), Ordering::Less);
        assert_eq!(compare("v1.0.0", "v1.0.0"), Ordering::Equal);
    }

    #[test]
    fn scan_and_newest_read_the_docs_root() {
        // ARRANGE
        let root = TempDir::new().expect("create temp dir");
        for line in ["v1.0.0-beta", "v1.1.0", "notes.toml"] {
            let directory = root.path().join("core");
            fs::create_dir_all(&directory).expect("create core dir");
            fs::write(directory.join(format!("{line}.toml")), "").expect("write line");
        }

        // ACT / ASSERT
        assert_eq!(newest(root.path()), Some("v1.1.0".to_owned()));
        assert_eq!(
            scan(root.path()),
            vec!["v1.0.0-beta".to_owned(), "v1.1.0".to_owned()]
        );
    }
}
