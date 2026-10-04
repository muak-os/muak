//! Blob info cache recording manifest layer lists and per-repository blob locations.

use core::time::Duration;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use oci::model::Descriptor;
use oci::reference::Image;
use serde::{Deserialize, Serialize};

/// Cache entry schema version.
const VERSION: u32 = 1;

/// Where a repository stands on a layer blob.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LayerLocation {
    /// The repository served this blob; the bytes may sit in the local store.
    Present,
    /// The repository answered "not found" for this blob within the negative TTL.
    Missing,
}

/// Versioned record of a resolved manifest's layer list.
#[derive(Deserialize, Serialize)]
struct LayersEntry {
    version: u32,
    layers: Vec<Descriptor>,
}

/// Versioned record of one blob's standing with one repository.
#[derive(Deserialize, Serialize)]
struct LocationEntry {
    version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    missing: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expires: Option<u64>,
}

/// Return the recorded layer list of a resolved manifest digest.
pub(crate) fn manifest_layers(
    root: Option<&Path>,
    image: &Image,
    manifest_digest: &str,
) -> Option<Vec<Descriptor>> {
    let path = entry_path(root, image, "layers", manifest_digest)?;
    let text = std::fs::read_to_string(path).ok()?;
    let entry = serde_json::from_str::<LayersEntry>(&text).ok()?;

    (entry.version == VERSION).then_some(entry.layers)
}

/// Record the layer list of a resolved manifest digest.
pub(crate) fn put_manifest_layers(
    root: Option<&Path>,
    image: &Image,
    manifest_digest: &str,
    layers: &[Descriptor],
) {
    write_entry(
        entry_path(root, image, "layers", manifest_digest),
        &LayersEntry {
            version: VERSION,
            layers: layers.to_vec(),
        },
    );
}

/// Where this repository stands on a layer blob, honoring the negative TTL.
pub(crate) fn layer_location(
    root: Option<&Path>,
    image: &Image,
    digest: &str,
) -> Option<LayerLocation> {
    let path = entry_path(root, image, "blobinfo", digest)?;
    let text = std::fs::read_to_string(path).ok()?;
    let entry = serde_json::from_str::<LocationEntry>(&text).ok()?;
    if entry.version != VERSION {
        return None;
    }

    match entry.missing {
        Some(true) => missing_fresh(entry.expires).then_some(LayerLocation::Missing),
        _ => Some(LayerLocation::Present),
    }
}

/// Record that this repository served a layer blob of `size` bytes.
pub(crate) fn put_blob_present(root: Option<&Path>, image: &Image, digest: &str, size: u64) {
    write_entry(
        entry_path(root, image, "blobinfo", digest),
        &LocationEntry {
            version: VERSION,
            size: Some(size),
            missing: None,
            expires: None,
        },
    );
}

/// Record that this repository does not serve a layer blob, for `ttl` seconds.
pub(crate) fn put_blob_missing(root: Option<&Path>, image: &Image, digest: &str, ttl: Duration) {
    write_entry(
        entry_path(root, image, "blobinfo", digest),
        &LocationEntry {
            version: VERSION,
            size: None,
            missing: Some(true),
            expires: Some(unix_now().saturating_add(ttl.as_secs())),
        },
    );
}

fn missing_fresh(expires: Option<u64>) -> bool {
    let Some(expires) = expires else {
        return false;
    };

    unix_now() < expires
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn entry_path(root: Option<&Path>, image: &Image, subdir: &str, digest: &str) -> Option<PathBuf> {
    let hash = digest.strip_prefix("sha256:")?;

    Some(
        root?
            .join(subdir)
            .join(&image.registry)
            .join(&image.name)
            .join(format!("{hash}.json")),
    )
}

fn write_entry(path: Option<PathBuf>, entry: &impl Serialize) {
    let Some(path) = path else { return };
    let Ok(json) = serde_json::to_string(entry) else {
        return;
    };
    let Some(parent) = path.parent() else { return };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }

    drop(std::fs::write(path, json));
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn test_image() -> Image {
        Image::parse("127.0.0.1:5000/org/image:latest")
    }

    fn descriptor(digest: &str, size: u64) -> Descriptor {
        Descriptor {
            media_type: Some("application/vnd.oci.image.layer.v1.tar".to_owned()),
            digest: digest.to_owned(),
            size,
            platform: None,
        }
    }

    #[test]
    fn manifest_layers_roundtrip() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let image = test_image();
        let layers = [descriptor("sha256:aaa", 12), descriptor("sha256:bbb", 34)];

        // ACT
        put_manifest_layers(Some(tmp.path()), &image, "sha256:ccc", &layers);
        let got = manifest_layers(Some(tmp.path()), &image, "sha256:ccc");

        // ASSERT
        assert_eq!(got, Some(layers.to_vec()));
    }

    #[test]
    fn manifest_layers_fall_back_cleanly_on_corrupt_entries() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let image = test_image();
        let entry = entry_path(Some(tmp.path()), &image, "layers", "sha256:aaa").expect("path");
        std::fs::create_dir_all(entry.parent().expect("entry dir")).expect("create entry dir");
        std::fs::write(&entry, "{not json").expect("write corrupt entry");

        // ACT
        let corrupt = manifest_layers(Some(tmp.path()), &image, "sha256:aaa");
        let absent = manifest_layers(Some(tmp.path()), &image, "sha256:missing");

        // ASSERT
        assert!(corrupt.is_none(), "corrupt entries must read as a miss");
        assert!(absent.is_none());
    }

    #[test]
    fn manifest_layers_fall_back_cleanly_on_version_mismatch() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let image = test_image();
        let entry = entry_path(Some(tmp.path()), &image, "layers", "sha256:aaa").expect("path");
        std::fs::create_dir_all(entry.parent().expect("entry dir")).expect("create entry dir");
        std::fs::write(&entry, r#"{"version":999,"layers":[]}"#).expect("write future entry");

        // ACT / ASSERT
        assert!(manifest_layers(Some(tmp.path()), &image, "sha256:aaa").is_none());
    }

    #[test]
    fn layer_locations_roundtrip_present_entries() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let image = test_image();

        // ACT
        put_blob_present(Some(tmp.path()), &image, "sha256:aaa", 42);
        let got = layer_location(Some(tmp.path()), &image, "sha256:aaa");

        // ASSERT
        assert_eq!(got, Some(LayerLocation::Present));
    }

    #[test]
    fn negative_locations_expire_after_the_ttl() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let image = test_image();

        // ACT
        put_blob_missing(
            Some(tmp.path()),
            &image,
            "sha256:aaa",
            Duration::from_mins(5),
        );
        let fresh = layer_location(Some(tmp.path()), &image, "sha256:aaa");
        put_blob_missing(Some(tmp.path()), &image, "sha256:old", Duration::ZERO);
        let expired = layer_location(Some(tmp.path()), &image, "sha256:old");

        // ASSERT
        assert_eq!(fresh, Some(LayerLocation::Missing));
        assert_eq!(
            expired, None,
            "expired negative entries must read as a miss"
        );
    }

    #[test]
    fn a_disabled_store_reads_and_writes_nothing() {
        // ARRANGE
        let image = test_image();

        // ACT
        put_manifest_layers(None, &image, "sha256:aaa", &[]);
        put_blob_present(None, &image, "sha256:aaa", 1);
        put_blob_missing(None, &image, "sha256:aaa", Duration::from_mins(1));

        // ASSERT
        assert!(manifest_layers(None, &image, "sha256:aaa").is_none());
        assert!(layer_location(None, &image, "sha256:aaa").is_none());
    }
}
