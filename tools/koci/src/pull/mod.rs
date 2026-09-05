//! Remote OCI registry pull orchestration.

use alloc::collections::BTreeMap;

use crate::arch::Arch;
use crate::error::Result;
use crate::image::manifest;
use crate::registry::auth::Access;
use crate::registry::session::Session;
use crate::runtime;

pub mod cache;
pub(crate) mod download;
pub mod entries;
pub(crate) mod layer;
pub(crate) mod paths;
pub(crate) mod resolve;
pub(crate) mod scan;

/// Manifest annotation carrying per-file sizes.
pub const SIZES_ANNOTATION: &str = "dev.muak.sizes";

/// Fetch the manifest annotations of the platform manifest matching `arch`.
///
/// Index-aware, cache-aware, and signature-verified. No blobs are downloaded.
///
/// # Errors
///
/// Returns an error if the manifest cannot be fetched or signature
/// verification fails.
pub fn annotations(
    reference: &str,
    arch: &Arch,
    pubkey_pem: Option<&str>,
) -> Result<BTreeMap<String, String>> {
    runtime::runtime()?.block_on(async {
        let session = Session::new(reference, Access::Pull, None).await?;
        let json = resolve::platform_manifest_json(&session, arch, pubkey_pem).await?;
        let parsed = manifest::parse(&json)?;

        Ok(parsed.annotations.unwrap_or_default().into_iter().collect())
    })
}

/// Stream file data from an OCI image.
///
/// # Errors
///
/// Returns an error if the image cannot be fetched, signature verification
/// fails, a layer cannot be decompressed, or the handler returns an error.
pub fn files<F>(reference: &str, arch: &Arch, pubkey_pem: Option<&str>, handler: F) -> Result<()>
where
    F: FnMut(entries::FileEntry<'_>) -> Result<()>,
{
    runtime::runtime()?.block_on(layer::files(reference, arch, pubkey_pem, handler))
}

#[cfg(test)]
mod tests {
    use alloc::collections::BTreeMap;

    use super::*;

    fn annotated(sizes: &[(&str, u64)]) -> BTreeMap<String, String> {
        let pairs = sizes
            .iter()
            .map(|&(path, size)| format!(r#""{path}":{size}"#))
            .collect::<Vec<_>>()
            .join(",");
        BTreeMap::from([(SIZES_ANNOTATION.to_owned(), format!(r"{{{pairs}}}"))])
    }

    #[test]
    fn annotations_carry_the_sizes_annotation_value_verbatim() {
        // ARRANGE
        let annotations = annotated(&[("vmlinuz", 12), ("cmdline", 3)]);

        // ACT / ASSERT
        assert_eq!(
            annotations.get(SIZES_ANNOTATION).map(String::as_str),
            Some(r#"{"vmlinuz":12,"cmdline":3}"#),
        );
    }
}
