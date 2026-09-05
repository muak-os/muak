//! Annotating pushed OCI images with per-file size metadata.

use crate::error::Result;
use crate::image::manifest;
use crate::pull;
use crate::registry::auth::Access;
use crate::registry::session::Session;
use crate::runtime;

/// Annotate an OCI image manifest in the registry with per-file sizes.
///
/// # Errors
///
/// Returns an error if the manifest or any layer blob cannot be fetched, or
/// the annotated manifest cannot be pushed.
pub fn manifest(reference: &str, exclude: &[String]) -> Result<()> {
    runtime::runtime()?.block_on(annotate(reference, exclude))
}

/// Annotate the referenced manifest (or every platform manifest of an index).
async fn annotate(reference: &str, exclude: &[String]) -> Result<()> {
    let session = Session::new(reference, Access::PullPush, None).await?;
    let manifest_json = fetch_manifest(&session, &session.image.manifest_ref).await?;
    let parsed = manifest::parse(&manifest_json)?;

    if parsed.manifests.is_empty() {
        return put_annotated(
            &session,
            &session.image.manifest_ref,
            &manifest_json,
            exclude,
        )
        .await;
    }

    for descriptor in &parsed.manifests {
        let platform_json = fetch_manifest(&session, &descriptor.digest).await?;
        put_annotated(&session, &descriptor.digest, &platform_json, exclude).await?;
    }

    Ok(())
}

/// Fetch a manifest by tag or digest reference.
async fn fetch_manifest(session: &Session, manifest_ref: &str) -> Result<String> {
    let url = manifest::build_url(&session.image, manifest_ref);

    manifest::fetch(&session.client, &url, session.authorization()).await
}

/// Scan a manifest's layers, inject the `dev.muak.sizes` annotation, and PUT
/// the manifest back under the same reference.
async fn put_annotated(
    session: &Session,
    manifest_ref: &str,
    manifest_json: &str,
    exclude: &[String],
) -> Result<()> {
    let parsed = manifest::parse(manifest_json)?;
    let sizes = pull::layer::entry_sizes(session, &parsed.layers, exclude).await?;
    eprintln!("Annotating {} file(s)", sizes.len());
    let sizes_json = serde_json::to_string(&sizes)?;
    let (body, content_type) =
        manifest::with_annotation(manifest_json, pull::SIZES_ANNOTATION, &sizes_json)?;

    manifest::put(session, manifest_ref, &content_type, body).await
}

#[cfg(test)]
mod tests {
    use alloc::collections::BTreeMap;

    #[test]
    fn serialized_sizes_map_is_a_compact_json_object() {
        // ARRANGE
        let sizes = BTreeMap::from([("vmlinuz".to_owned(), 12_345_u64)]);

        // ACT
        let json = serde_json::to_string(&sizes).expect("serialize sizes");

        // ASSERT
        assert_eq!(json, r#"{"vmlinuz":12345}"#);
    }
}
