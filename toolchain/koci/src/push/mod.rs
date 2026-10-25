//! OCI image push.

use std::path::PathBuf;

use bytes::Bytes;
use oci::arch::Arch;
use oci::digest::sha256_hex;
use oci::media::OCI_MANIFEST_MEDIA_TYPE;
use oci_client::auth::Access;
use oci_client::blob::report::BlobProgress;
use oci_client::client::Client;

use crate::error::Result;
use crate::image::{config, manifest, upload};
use crate::progress::{ByteSample, Event, Operation, Phase, Reporter, Unit};
use crate::registry;
use crate::runtime;

mod entry;
mod layer;

/// One file packed into a pushed image, created via [`Entry::parse`].
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
    /// Tags the manifest was published under.
    pub tags: Vec<String>,
}

/// Pack `entries` into one uncompressed tar layer and push a scratch image.
///
/// # Errors
///
/// Returns an error when entries are invalid, the session cannot be established,
/// or any registry operation fails.
pub fn files(
    image: &str,
    tags: &[String],
    arch: &Arch,
    entries: &[Entry],
    reporter: &dyn Reporter,
) -> Result<Pushed> {
    runtime::runtime()?.block_on(push_files(image, tags, arch, entries, reporter))
}

/// Packs entries into an eStargz layer, uploading the config and layer blobs.
/// Returns the platform manifest bytes and digest without pushing any tag.
///
/// # Errors
///
/// Returns an error when a source cannot be read, an entry is invalid, or a
/// registry upload fails.
pub(crate) async fn pack(
    client: &Client,
    arch: Arch,
    entries: &[Entry],
    reporter: &dyn Reporter,
) -> Result<(Bytes, String)> {
    let layer = layer::build(entries)?;
    let packed_bytes = u64::try_from(layer.bytes.len()).unwrap_or(u64::MAX);
    reporter.report(Event::Packed {
        entries: entries.len(),
        bytes: packed_bytes,
    });
    reporter.report(Event::PhaseStarted {
        phase: Phase::Upload,
        label: "Uploading layer",
        total: packed_bytes,
        unit: Unit::Bytes,
    });
    let config = config::scratch(arch, &layer.uncompressed)?;
    let blob_reporter = UploadReporter(reporter);
    upload::put_blob(client, &config.digest, config.bytes.clone(), &blob_reporter).await?;
    upload::put_blob(
        client,
        &layer.digest,
        Bytes::from(layer.bytes.clone()),
        &blob_reporter,
    )
    .await?;
    let bytes = manifest::build(&config, &[manifest::layer_descriptor(&layer)], None)?;
    let digest = format!("sha256:{}", sha256_hex(&bytes));

    Ok((bytes, digest))
}

struct UploadReporter<'a>(&'a dyn Reporter);

impl BlobProgress for UploadReporter<'_> {
    fn progress(&self, sent: u64, total: u64) {
        self.0.bytes(ByteSample {
            phase: Phase::Upload,
            index: None,
            current: sent,
            total,
        });
    }

    fn skipped(&self, digest: &str) {
        self.0.report(Event::ItemSkipped {
            phase: Phase::Upload,
            digest,
        });
    }
}

async fn push_files(
    image: &str,
    tags: &[String],
    arch: &Arch,
    entries: &[Entry],
    reporter: &dyn Reporter,
) -> Result<Pushed> {
    reporter.report(Event::Started {
        operation: Operation::Push,
        subject: image,
    });
    let client = registry::connect(image, Access::PullPush).await?;
    let tags = upload::effective_tags(&client, tags)?;
    Entry::validate(entries)?;

    let (bytes, digest) = pack(&client, *arch, entries, reporter).await?;
    for tag in &tags {
        oci_client::manifest::put(&client, tag, OCI_MANIFEST_MEDIA_TYPE, bytes.clone()).await?;
        reporter.report(Event::ManifestPushed { reference: tag });
    }
    reporter.report(Event::Done {
        digest: &digest,
        tags: &tags,
    });

    Ok(Pushed { digest, tags })
}

#[cfg(test)]
mod tests {
    use core::fmt::Write as _;
    use std::io::Read as _;
    use std::io::Write as _;
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::thread;

    use oci::arch::Arch;
    use tempfile::TempDir;

    use super::*;
    use crate::image::Blob;
    use crate::progress::Noop;
    use crate::stargz::LayerBlob;

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
    fn parse_reads_explicit_and_default_archive_paths() {
        // ARRANGE
        let workspace = TempDir::new().expect("create temp dir");
        let source = workspace.path().join("catalog.toml");
        std::fs::write(&source, b"").expect("write test file");
        let explicit = format!("{}:docs/catalog.toml", source.display());
        let default = source.display().to_string();

        // ACT
        let with_path = Entry::parse(&explicit).expect("parse explicit spec");
        let with_default = Entry::parse(&default).expect("parse default spec");

        // ASSERT
        assert_eq!(with_path.path, "docs/catalog.toml");
        assert_eq!(with_path.source, source);
        assert_eq!(with_default.path, "catalog.toml");
    }

    #[test]
    fn parse_rejects_empty_and_unnameable_sources() {
        // ARRANGE / ACT / ASSERT
        let error = Entry::parse(":docs/catalog.toml").expect_err("empty source should fail");
        assert!(matches!(error, crate::error::KociError::PushError(_)));

        let error = Entry::parse("/").expect_err("unnameable source should fail");
        assert!(matches!(error, crate::error::KociError::PushError(_)));
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
        assert!(Entry::validate(&[absolute]).is_err());
        assert!(Entry::validate(&[traversing]).is_err());
        assert!(Entry::validate(&[entry.clone(), entry]).is_err());
    }

    #[test]
    fn manifest_lists_config_and_layer_descriptors() {
        // ARRANGE
        let config = Blob {
            digest: "sha256:aaa".to_owned(),
            size: 3,
            bytes: Bytes::new(),
        };
        let layer = LayerBlob {
            digest: "sha256:bbb".to_owned(),
            size: 5,
            bytes: Vec::new(),
            uncompressed: "sha256:ccc".to_owned(),
            uncompressed_size: 7,
            toc_digest: "sha256:ddd".to_owned(),
        };

        // ACT
        let bytes = manifest::build(&config, &[manifest::layer_descriptor(&layer)], None)
            .expect("build manifest");

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
        let pushed = files(
            &image,
            &[],
            &Arch::Amd64,
            std::slice::from_ref(&entry),
            &Noop,
        )
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
        let error = files(
            &image,
            &[],
            &Arch::Amd64,
            std::slice::from_ref(&entry),
            &Noop,
        )
        .expect_err("push should fail");

        // ASSERT
        assert!(matches!(
            error,
            crate::error::KociError::Client(oci_client::error::ClientError::Push(_))
        ));
    }
}
