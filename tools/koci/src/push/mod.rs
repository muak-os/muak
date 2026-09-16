//! OCI image push.

use std::path::PathBuf;

use hyper::body::Bytes;

use crate::arch::Arch;
use crate::digest::sha256_hex;
use crate::error::{KociError, Result};
use crate::image::manifest;
use crate::registry::auth::Access;
use crate::registry::session::Session;
use crate::registry::{OCI_CONFIG_MEDIA_TYPE, OCI_LAYER_MEDIA_TYPE, OCI_MANIFEST_MEDIA_TYPE};
use crate::runtime;

mod entry;
mod layer;
pub(crate) mod upload;

/// One file packed into a pushed image, created via [`parse_entry`].
#[derive(Debug, Clone)]
pub struct Entry {
    /// Path of the file inside the image layer (relative, e.g. `catalog.toml`).
    pub path: String,
    /// Local file providing the entry bytes.
    pub source: PathBuf,
}

/// Identity of a successfully pushed image.
#[derive(Debug, Clone)]
pub struct Pushed {
    /// Digest of the pushed image manifest.
    pub digest: String,
}

/// An assembled blob with its digest and byte length.
#[derive(Debug)]
pub(crate) struct Blob {
    /// `sha256:...` digest of the blob bytes.
    pub(crate) digest: String,
    /// Byte length of the blob.
    pub(crate) size: u64,
    /// The blob bytes.
    pub(crate) bytes: Bytes,
}

/// Parse a `PATH[:ARCHIVE_PATH]` entry specification such as `catalog.toml`.
///
/// # Errors
///
/// Returns an error when the source path or the derived archive path is empty.
pub fn parse_entry(spec: &str) -> Result<Entry> {
    entry::parse(spec)
}

/// Pack `entries` into one uncompressed tar layer and push a scratch image.
///
/// # Errors
///
/// Returns an error when entries are invalid, the session cannot be established, or any registry operation fails.
pub fn files(image: &str, tags: &[String], arch: &Arch, entries: &[Entry]) -> Result<Pushed> {
    runtime::runtime()?.block_on(push_files(image, tags, arch, entries))
}

async fn push_files(
    image: &str,
    tags: &[String],
    arch: &Arch,
    entries: &[Entry],
) -> Result<Pushed> {
    let session = Session::new(image, Access::PullPush, None).await?;
    let tags = effective_tags(&session, tags)?;
    entry::validate(entries)?;

    let layer = layer::build(entries)?;
    let config = config_blob(*arch, &layer)?;
    upload::blob(&session, &config.digest, config.bytes.clone()).await?;
    upload::blob(&session, &layer.digest, layer.bytes.clone()).await?;

    let bytes = manifest_bytes(&config, &layer)?;
    for tag in &tags {
        manifest::put(&session, tag, OCI_MANIFEST_MEDIA_TYPE, bytes.clone()).await?;
        eprintln!("Pushed manifest to {image}:{tag}");
    }

    Ok(Pushed {
        digest: format!("sha256:{}", sha256_hex(&bytes)),
    })
}

fn effective_tags(session: &Session, tags: &[String]) -> Result<Vec<String>> {
    if !tags.is_empty() {
        return Ok(tags.to_vec());
    }
    if session.image.manifest_ref.starts_with("sha256:") {
        return Err(KociError::InvalidOciFormat(
            "digest reference carries no tag to push; pass --tag".to_owned(),
        ));
    }

    Ok(vec![session.image.manifest_ref.clone()])
}

fn config_blob(arch: Arch, layer: &Blob) -> Result<Blob> {
    blob_from_json(&serde_json::json!({
        "architecture": arch.as_str(),
        "os": "linux",
        "rootfs": { "type": "layers", "diff_ids": [layer.digest] },
    }))
}

fn blob_from_json(value: &serde_json::Value) -> Result<Blob> {
    let bytes = Bytes::from(serde_json::to_vec(value)?);
    let digest = format!("sha256:{}", sha256_hex(&bytes));

    Ok(Blob {
        digest,
        size: blob_size(bytes.len())?,
        bytes,
    })
}

/// Convert a byte length into a descriptor size.
pub(crate) fn blob_size(len: usize) -> Result<u64> {
    u64::try_from(len)
        .map_err(|error| KociError::PushError(format!("blob size out of range: {error}")))
}

fn manifest_bytes(config: &Blob, layer: &Blob) -> Result<Bytes> {
    let body = serde_json::json!({
        "schemaVersion": 2,
        "mediaType": OCI_MANIFEST_MEDIA_TYPE,
        "config": descriptor(OCI_CONFIG_MEDIA_TYPE, config),
        "layers": [descriptor(OCI_LAYER_MEDIA_TYPE, layer)],
    });

    Ok(Bytes::from(serde_json::to_vec(&body)?))
}

fn descriptor(media_type: &str, blob: &Blob) -> serde_json::Value {
    serde_json::json!({ "mediaType": media_type, "digest": blob.digest, "size": blob.size })
}

#[cfg(test)]
mod tests {
    use core::fmt::Write as _;
    use std::io::Read as _;
    use std::io::Write as _;
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::thread;

    use tempfile::TempDir;

    use super::*;
    use crate::arch::Arch;

    struct TestServer {
        address: String,
        handle: Option<thread::JoinHandle<()>>,
    }

    impl TestServer {
        fn spawn(responses: &[String]) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
            let address = listener.local_addr().expect("read test server address");
            let responses = responses.to_vec();
            let handle = thread::spawn(move || serve_responses(&listener, &responses));

            Self {
                address: address.to_string(),
                handle: Some(handle),
            }
        }
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            join_server(self.handle.take());
        }
    }

    fn serve_responses(listener: &TcpListener, responses: &[String]) {
        for response in responses {
            serve_response(listener, response);
        }
    }

    fn serve_response(listener: &TcpListener, response: &str) {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        let mut request = [0_u8; 4096];
        let _: usize = stream.read(&mut request).unwrap_or(0);
        stream
            .write_all(response.as_bytes())
            .expect("write test response");
        stream.flush().expect("flush test response");
    }

    fn join_server(handle: Option<thread::JoinHandle<()>>) {
        if let Some(handle) = handle {
            handle.join().expect("join test server thread");
        }
    }

    fn raw(status: &str, extra_headers: &[&str], body: &str) -> String {
        let mut response = format!("HTTP/1.1 {status}\r\n");
        for header in extra_headers {
            writeln!(response, "{header}").expect("write test response header");
        }
        write!(
            response,
            "Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .expect("write test response body");

        response
    }

    fn entry_in(dir: &TempDir, name: &str, contents: &[u8]) -> Entry {
        let source = dir.path().join(name);
        std::fs::write(&source, contents).expect("write test entry file");

        Entry {
            path: name.to_owned(),
            source,
        }
    }

    #[test]
    fn parse_entry_reads_explicit_and_default_archive_paths() {
        // ARRANGE
        let workspace = TempDir::new().expect("create temp dir");
        let source = workspace.path().join("catalog.toml");
        std::fs::write(&source, b"").expect("write test file");
        let explicit = format!("{}:docs/catalog.toml", source.display());
        let default = source.display().to_string();

        // ACT
        let with_path = parse_entry(&explicit).expect("parse explicit spec");
        let with_default = parse_entry(&default).expect("parse default spec");

        // ASSERT
        assert_eq!(with_path.path, "docs/catalog.toml");
        assert_eq!(with_path.source, source);
        assert_eq!(with_default.path, "catalog.toml");
    }

    #[test]
    fn parse_entry_rejects_empty_and_unnameable_sources() {
        // ARRANGE / ACT / ASSERT
        let error = parse_entry(":docs/catalog.toml").expect_err("empty source should fail");
        assert!(matches!(error, KociError::InvalidOciFormat(_)));

        let error = parse_entry("/").expect_err("unnameable source should fail");
        assert!(matches!(error, KociError::InvalidOciFormat(_)));
    }

    #[test]
    fn validate_rejects_bad_and_duplicate_archive_paths() {
        // ARRANGE
        let workspace = TempDir::new().expect("create temp dir");
        let entry = entry_in(&workspace, "a.toml", b"");
        let absolute = Entry {
            path: "/abs.toml".to_owned(),
            source: PathBuf::from("b.toml"),
        };
        let traversing = Entry {
            path: "../up.toml".to_owned(),
            source: PathBuf::from("c.toml"),
        };

        // ACT / ASSERT
        assert!(entry::validate(&[absolute]).is_err());
        assert!(entry::validate(&[traversing]).is_err());
        assert!(entry::validate(&[entry.clone(), entry]).is_err());
    }

    #[test]
    fn manifest_bytes_lists_config_and_layer_descriptors() {
        // ARRANGE
        let config = Blob {
            digest: "sha256:aaa".to_owned(),
            size: 3,
            bytes: Bytes::new(),
        };
        let layer = Blob {
            digest: "sha256:bbb".to_owned(),
            size: 5,
            bytes: Bytes::new(),
        };

        // ACT
        let bytes = manifest_bytes(&config, &layer).expect("build manifest");

        // ASSERT
        let parsed: serde_json::Value =
            serde_json::from_slice(&bytes).expect("parse manifest json");
        assert_eq!(
            parsed.get("mediaType").and_then(serde_json::Value::as_str),
            Some(OCI_MANIFEST_MEDIA_TYPE)
        );
        assert_eq!(
            parsed
                .get("config")
                .and_then(|config| config.get("digest"))
                .and_then(serde_json::Value::as_str),
            Some("sha256:aaa")
        );
        assert_eq!(
            parsed
                .get("layers")
                .and_then(serde_json::Value::as_array)
                .and_then(|layers| layers.first())
                .and_then(|layer| layer.get("digest"))
                .and_then(serde_json::Value::as_str),
            Some("sha256:bbb")
        );
    }

    #[test]
    fn files_pushes_scratch_image_through_the_registry_flow() {
        // ARRANGE
        let workspace = TempDir::new().expect("create temp dir");
        let entry = entry_in(&workspace, "catalog.toml", b"api_version = 1\n");
        let server = TestServer::spawn(&[
            raw("200 OK", &[], ""),                                            // auth ping
            raw("404 Not Found", &[], ""),                                     // config HEAD
            raw("202 Accepted", &["Location: /v2/repo/blobs/uploads/u1"], ""), // config POST
            raw("201 Created", &[], ""),                                       // config PUT
            raw("404 Not Found", &[], ""),                                     // layer HEAD
            raw("202 Accepted", &["Location: /v2/repo/blobs/uploads/u2"], ""), // layer POST
            raw("201 Created", &[], ""),                                       // layer PUT
            raw("201 Created", &["Docker-Content-Digest: sha256:abc"], ""),    // manifest PUT
        ]);
        let image = format!("{}/repo:v1", server.address);

        // ACT
        let pushed = files(&image, &[], &Arch::Amd64, std::slice::from_ref(&entry))
            .expect("push should succeed");

        // ASSERT
        assert!(pushed.digest.starts_with("sha256:"));
        assert_eq!(pushed.digest.len(), "sha256:".len() + 64);
    }

    #[test]
    fn files_reports_upload_failures() {
        // ARRANGE
        let workspace = TempDir::new().expect("create temp dir");
        let entry = entry_in(&workspace, "catalog.toml", b"api_version = 1\n");
        let server = TestServer::spawn(&[
            raw("200 OK", &[], ""),                    // auth ping
            raw("404 Not Found", &[], ""),             // config HEAD
            raw("500 Internal Server Error", &[], ""), // config POST fails
        ]);
        let image = format!("{}/repo:v1", server.address);

        // ACT
        let error = files(&image, &[], &Arch::Amd64, std::slice::from_ref(&entry))
            .expect_err("push should fail");

        // ASSERT
        assert!(matches!(error, KociError::PushError(_)));
    }
}
