//! Seeding a new release line.

use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::repository;
use crate::schema::documents::{CoreDocument, Document};
use crate::schema::kinds::{CORE_API_VERSION, Kind};
use crate::schema::parse::validate_release;

/// Seed `release` under `root`, optionally carrying overlay and extension
/// documents over from a previous line.
///
/// # Errors
///
/// Returns an error when the root is unusable or a document cannot be written.
pub fn run(root: &Path, release: &str, from: Option<&str>) -> Result<Vec<PathBuf>> {
    repository::require_root(root)?;
    validate_release(release)?;
    let mut written = vec![seed_core(root, release)?];
    for kind in [Kind::Overlays, Kind::Extensions] {
        if let Some(path) = carry_over(kind, root, from, release)? {
            written.push(path);
        }
    }

    Ok(written)
}

fn seed_core(root: &Path, release: &str) -> Result<PathBuf> {
    let document = Document::Core(CoreDocument {
        api_version: CORE_API_VERSION.to_owned(),
        release: release.to_owned(),
        kernels: Vec::new(),
        stub: None,
        installer: None,
    });

    repository::write(&document, root, release)
}

fn carry_over(
    kind: Kind,
    root: &Path,
    from: Option<&str>,
    release: &str,
) -> Result<Option<PathBuf>> {
    let Some(previous) = from else {
        return Ok(None);
    };
    if !repository::document_path(kind, root, previous).exists() {
        eprintln!(
            "No {} document for {previous}; seeding {release} empty",
            kind.dir()
        );

        return Ok(None);
    }

    let mut document = repository::load(kind, root, previous)?;
    document.set_release(release.to_owned());

    Ok(Some(repository::write(&document, root, release)?))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;
    use crate::schema::parse::from_toml;

    #[test]
    fn init_writes_core_skeleton_and_carries_previous_lines() {
        // ARRANGE
        let root = TempDir::new().expect("create temp root");
        let previous = from_toml(
            Kind::Overlays,
            b"api_version = \"muak.dev/catalog/overlays/v1\"\nrelease = \"v1.2.2\"\n",
            "v1.2.2",
        )
        .expect("parse previous overlays document");
        repository::write(&previous, root.path(), "v1.2.2").expect("write previous");

        // ACT
        let written = run(root.path(), "v1.2.3", Some("v1.2.2")).expect("init should succeed");

        // ASSERT
        assert_eq!(written.len(), 2);
        let core = fs::read_to_string(written.first().expect("core path")).expect("read core");
        assert!(core.contains("release = \"v1.2.3\""));
        assert!(core.contains("muak.dev/catalog/core/v1"));
        let overlays =
            fs::read_to_string(written.last().expect("overlays path")).expect("read overlays");
        assert!(overlays.contains("release = \"v1.2.3\""));
    }

    #[test]
    fn init_without_previous_line_seeds_core_only() {
        // ARRANGE
        let root = TempDir::new().expect("create temp root");

        // ACT
        let written = run(root.path(), "v1.0.0", Some("v0.9.0")).expect("init should succeed");

        // ASSERT
        assert_eq!(written.len(), 1);
        assert!(written.first().expect("core path").exists());
    }
}
