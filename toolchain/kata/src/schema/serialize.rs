//! Canonical serialization of catalog documents.

use crate::error;
use crate::schema::documents::Document;

/// Serializes `document` canonically: entries sorted, fields in schema order.
///
/// # Errors
///
/// Returns an error when serialization fails.
pub fn canonical(document: &Document) -> error::DocumentResult<String> {
    let mut sorted = document.clone();
    sort_entries(&mut sorted);
    let text = match sorted {
        Document::Core(ref core) => toml::to_string_pretty(core),
        Document::Overlays(ref overlays) => toml::to_string_pretty(overlays),
        Document::Extensions(ref extensions) => toml::to_string_pretty(extensions),
    };

    text.map_err(|error| error::DocumentError::Serialize(error.to_string()))
}

fn sort_entries(document: &mut Document) {
    match *document {
        Document::Core(ref mut core) => {
            core.kernels
                .sort_by(|left, right| left.source.cmp(&right.source));
        }
        Document::Overlays(ref mut overlays) => {
            overlays
                .overlays
                .sort_by(|left, right| left.name.cmp(&right.name));
        }
        Document::Extensions(ref mut extensions) => {
            extensions
                .extensions
                .sort_by(|left, right| left.name.cmp(&right.name));
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::schema::kinds::Kind;
    use crate::schema::parse::from_toml;

    #[test]
    fn canonical_sorts_entries_deterministically() {
        // ARRANGE
        let unsorted = r#"api_version = "muak.dev/catalog/overlays/v1"
release = "v1.2.3"

[[overlays]]
name = "rpi_5"
source = "muak-os/sbc-raspberrypi"
repository = "sbc/raspberrypi"
tag = "v0.4.0"
digest = "sha256:5555"

[[overlays]]
name = "rpi_generic"
source = "muak-os/sbc-raspberrypi"
repository = "sbc/raspberrypi"
tag = "v0.4.0"
digest = "sha256:5555"
"#;
        let document =
            from_toml(Kind::Overlays, unsorted.as_bytes(), "v1.2.3").expect("parse overlays");

        // ACT
        let canonical = super::canonical(&document).expect("canonical serialization");

        // ASSERT
        let generic = canonical.find("name = \"rpi_generic\"").expect("generic");
        let five = canonical.find("name = \"rpi_5\"").expect("rpi_5");
        assert!(five < generic, "entries must be sorted by name");
    }
}
