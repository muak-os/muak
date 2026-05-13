//! Remote OCI registry pull orchestration.

use std::path::Path;

use crate::error::{KociError, Result};
use crate::image::manifest;
use crate::image::{ImageReference, OciDescriptor};
use crate::registry::auth::fetch_auth_token;
use crate::registry::http::{HttpClient, build_client};
use crate::sign::verify;

pub(crate) mod layer;

/// Maximum number of concurrent layer downloads.
const MAX_CONCURRENT_DOWNLOADS: usize = 8;

type DownloadJoinSet = tokio::task::JoinSet<Result<(usize, Vec<u8>, Option<String>)>>;

/// Pull an OCI image and extract all layers to `dest`.
pub(crate) async fn pull_to_dir(
    reference: &str,
    arch: &str,
    dest: &Path,
    signature_key: Option<&str>,
) -> Result<()> {
    let image_ref = ImageReference::parse(reference);
    let client = build_client()?;
    let target_arch = arch.to_string();

    let token = fetch_auth_token(&client, &image_ref.registry, &image_ref.name).await?;
    let manifest_url = manifest::build_url(&image_ref, &image_ref.manifest_ref);
    let manifest_json = manifest::fetch(&client, &manifest_url, token.as_deref()).await?;
    let manifest = manifest::parse(&manifest_json)?;

    let layers = if !manifest.manifests.is_empty() {
        verify::check_signature(&manifest_json, signature_key).await?;

        let selected = manifest::select_platform(&manifest.manifests, &target_arch)?;
        let platform_url = manifest::build_url(&image_ref, &selected.digest);
        let platform_json = manifest::fetch(&client, &platform_url, token.as_deref()).await?;

        verify::check_signature(&platform_json, signature_key).await?;

        manifest::parse(&platform_json)?.layers
    } else {
        verify::check_signature(&manifest_json, signature_key).await?;
        manifest.layers
    };

    download_and_extract_layers(&client, &image_ref, &layers, token.as_deref(), dest).await
}

/// Download all layers with bounded parallelism, then apply them in manifest order.
async fn download_and_extract_layers(
    client: &HttpClient,
    image_ref: &ImageReference,
    layers: &[OciDescriptor],
    token: Option<&str>,
    dest: &Path,
) -> Result<()> {
    let token = token.map(str::to_string);
    let mut downloaded: Vec<Option<(Vec<u8>, Option<String>)>> = vec![None; layers.len()];
    let mut join_set: DownloadJoinSet = tokio::task::JoinSet::new();
    let mut iter = layers.iter().enumerate();

    fill_download_slots(&mut join_set, &mut iter, client, image_ref, &token);

    while let Some(result) = join_set.join_next().await {
        match result {
            Ok(Ok((index, bytes, media_type))) => {
                downloaded[index] = Some((bytes, media_type));
                fill_download_slots(&mut join_set, &mut iter, client, image_ref, &token);
            }
            Ok(Err(error)) => {
                join_set.abort_all();
                return Err(error);
            }
            Err(error) => {
                join_set.abort_all();
                return Err(KociError::DownloadError(format!(
                    "Layer download task panicked: {error}"
                )));
            }
        }
    }

    for (index, entry) in downloaded.into_iter().enumerate() {
        let (bytes, media_type) = entry.ok_or_else(|| {
            KociError::DownloadError(format!("missing download result for layer {index}"))
        })?;
        tokio::task::spawn_blocking({
            let dest = dest.to_path_buf();
            move || layer::extract_archive(&bytes, media_type.as_deref(), &dest)
        })
        .await
        .map_err(|e| KociError::LayerExtractionError(format!("layer {index}: {e}")))??;
    }

    Ok(())
}

fn fill_download_slots<'a>(
    join_set: &mut DownloadJoinSet,
    iter: &mut impl Iterator<Item = (usize, &'a OciDescriptor)>,
    client: &HttpClient,
    image_ref: &ImageReference,
    token: &Option<String>,
) {
    while join_set.len() < MAX_CONCURRENT_DOWNLOADS {
        let Some((index, layer_desc)) = iter.next() else {
            return;
        };
        let client = client.clone();
        let image_ref = image_ref.clone();
        let digest = layer_desc.digest.clone();
        let media_type = layer_desc.media_type.clone();
        let token = token.clone();
        join_set.spawn(async move {
            let bytes =
                layer::download_blob(&client, &image_ref, &digest, token.as_deref()).await?;
            Ok((index, bytes, media_type))
        });
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::io::{Error as IoError, ErrorKind, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::{Duration, Instant};

    use flate2::Compression;
    use flate2::write::GzEncoder;
    use tar::{Builder, Header};
    use tempfile::TempDir;

    use super::*;
    use crate::registry::http::build_client;

    type RouteKey = (String, String);

    #[derive(Clone)]
    struct HttpResponse {
        status: u16,
        content_type: &'static str,
        body: Vec<u8>,
        delay: Duration,
    }

    impl HttpResponse {
        fn manifest(body: Vec<u8>) -> Self {
            Self {
                status: 200,
                content_type: "application/vnd.oci.image.manifest.v1+json",
                body,
                delay: Duration::ZERO,
            }
        }

        fn blob(body: Vec<u8>) -> Self {
            Self {
                status: 200,
                content_type: "application/octet-stream",
                body,
                delay: Duration::ZERO,
            }
        }

        fn not_found() -> Self {
            Self {
                status: 404,
                content_type: "text/plain",
                body: b"missing".to_vec(),
                delay: Duration::ZERO,
            }
        }

        fn with_delay(mut self, delay: Duration) -> Self {
            self.delay = delay;
            self
        }
    }

    struct TestRegistry {
        address: String,
        connections: Arc<Mutex<Vec<thread::JoinHandle<()>>>>,
        shutdown: Arc<AtomicBool>,
        handle: Option<thread::JoinHandle<()>>,
    }

    impl TestRegistry {
        fn start(routes: HashMap<RouteKey, HttpResponse>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind test registry");
            listener
                .set_nonblocking(true)
                .expect("set test registry nonblocking");

            let address = listener
                .local_addr()
                .expect("get test registry address")
                .to_string();
            let routes = Arc::new(routes);
            let connections = Arc::new(Mutex::new(Vec::new()));
            let shutdown = Arc::new(AtomicBool::new(false));
            let thread_routes = Arc::clone(&routes);
            let thread_connections = Arc::clone(&connections);
            let thread_shutdown = Arc::clone(&shutdown);

            let handle = thread::spawn(move || {
                loop {
                    if thread_shutdown.load(Ordering::SeqCst) {
                        break;
                    }

                    let stream = match listener.accept() {
                        Ok((stream, _)) => stream,
                        Err(error) if error.kind() == ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(10));
                            continue;
                        }
                        Err(_) => break,
                    };

                    let thread_routes = Arc::clone(&thread_routes);
                    let connection_handle = thread::spawn(move || {
                        let _ = handle_connection(stream, &thread_routes);
                    });
                    thread_connections
                        .lock()
                        .expect("lock test registry connection handles")
                        .push(connection_handle);
                }
            });

            Self {
                address,
                connections,
                shutdown,
                handle: Some(handle),
            }
        }

        fn reference(&self, repository: &str, tag: &str) -> String {
            format!("{}/{repository}:{tag}", self.address)
        }
    }

    impl Drop for TestRegistry {
        fn drop(&mut self) {
            self.shutdown.store(true, Ordering::SeqCst);
            if let Some(handle) = self.handle.take() {
                handle.join().expect("join test registry thread");
            }

            let connection_handles = std::mem::take(
                &mut *self
                    .connections
                    .lock()
                    .expect("lock test registry connection handles"),
            );
            for handle in connection_handles {
                handle.join().expect("join test connection thread");
            }
        }
    }

    fn handle_connection(
        mut stream: TcpStream,
        routes: &HashMap<RouteKey, HttpResponse>,
    ) -> std::result::Result<(), IoError> {
        let request = read_request(&mut stream)?;
        let response = routes
            .get(&request)
            .cloned()
            .unwrap_or_else(HttpResponse::not_found);
        write_response(&mut stream, &response)
    }

    fn read_request(stream: &mut TcpStream) -> std::result::Result<RouteKey, IoError> {
        let mut buffer = Vec::new();
        let mut chunk = [0_u8; 1024];

        loop {
            let read = stream.read(&mut chunk)?;
            if read == 0 {
                return Err(IoError::new(
                    ErrorKind::UnexpectedEof,
                    "connection closed before headers completed",
                ));
            }
            buffer.extend_from_slice(&chunk[..read]);

            if let Some(header_end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
                let header_text = std::str::from_utf8(&buffer[..header_end])
                    .map_err(|error| IoError::new(ErrorKind::InvalidData, error))?;
                let mut parts = header_text
                    .lines()
                    .next()
                    .ok_or_else(|| IoError::new(ErrorKind::InvalidData, "missing request line"))?
                    .split_whitespace();
                let method = parts
                    .next()
                    .ok_or_else(|| IoError::new(ErrorKind::InvalidData, "missing request method"))?
                    .to_string();
                let path = parts
                    .next()
                    .ok_or_else(|| IoError::new(ErrorKind::InvalidData, "missing request path"))?
                    .to_string();
                return Ok((method, path));
            }
        }
    }

    fn write_response(
        stream: &mut TcpStream,
        response: &HttpResponse,
    ) -> std::result::Result<(), IoError> {
        if !response.delay.is_zero() {
            thread::sleep(response.delay);
        }

        let reason = match response.status {
            200 => "OK",
            404 => "Not Found",
            _ => "Error",
        };

        let headers = format!(
            "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nContent-Type: {}\r\nConnection: close\r\n\r\n",
            response.status,
            reason,
            response.body.len(),
            response.content_type,
        );

        stream.write_all(headers.as_bytes())?;
        stream.write_all(&response.body)?;
        stream.flush()?;
        Ok(())
    }

    fn image_reference(registry: &TestRegistry) -> ImageReference {
        ImageReference {
            registry: registry.address.clone(),
            name: "repo".to_string(),
            manifest_ref: "test".to_string(),
        }
    }

    fn descriptor(digest: &str, media_type: Option<&str>) -> OciDescriptor {
        OciDescriptor {
            media_type: media_type.map(str::to_string),
            digest: digest.to_string(),
            platform: None,
        }
    }

    fn layer_archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let encoder = GzEncoder::new(Vec::new(), Compression::default());
        let mut archive = Builder::new(encoder);

        for (path, bytes) in entries {
            let mut header = Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            archive
                .append_data(&mut header, path, *bytes)
                .expect("append layer data");
        }

        archive
            .into_inner()
            .expect("finish layer archive")
            .finish()
            .expect("finish gzip archive")
    }

    fn sha256_digest(bytes: &[u8]) -> String {
        format!("sha256:{}", crate::digest::sha256_hex(bytes))
    }

    fn manifest_json(layer_digest: &str, layer_size: usize, media_type: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 2,
            "mediaType": "application/vnd.oci.image.manifest.v1+json",
            "config": {
                "mediaType": "application/vnd.oci.image.config.v1+json",
                "digest": "sha256:1111111111111111111111111111111111111111111111111111111111111111",
                "size": 1,
            },
            "layers": [{
                "mediaType": media_type,
                "digest": layer_digest,
                "size": layer_size,
            }],
        }))
        .expect("serialize manifest json")
    }

    #[tokio::test]
    async fn download_and_extract_layers_applies_layers_in_manifest_order() {
        // ARRANGE
        let first_layer = layer_archive(&[("etc/message", b"first\n")]);
        let second_layer = layer_archive(&[("etc/.wh.message", b"")]);
        let first_digest = sha256_digest(&first_layer);
        let second_digest = sha256_digest(&second_layer);
        let registry = TestRegistry::start(HashMap::from([
            (
                ("GET".to_string(), format!("/v2/repo/blobs/{first_digest}")),
                HttpResponse::blob(first_layer),
            ),
            (
                ("GET".to_string(), format!("/v2/repo/blobs/{second_digest}")),
                HttpResponse::blob(second_layer),
            ),
        ]));
        let client = build_client().expect("build HTTP client");
        let output = TempDir::new().expect("create temp dir");
        let layers = vec![
            descriptor(
                &first_digest,
                Some("application/vnd.oci.image.layer.v1.tar+gzip"),
            ),
            descriptor(
                &second_digest,
                Some("application/vnd.oci.image.layer.v1.tar+gzip"),
            ),
        ];

        // ACT
        download_and_extract_layers(
            &client,
            &image_reference(&registry),
            &layers,
            None,
            output.path(),
        )
        .await
        .expect("download and extract layers");

        // ASSERT
        assert!(!output.path().join("etc/message").exists());
    }

    #[tokio::test]
    async fn download_and_extract_layers_downloads_blobs_in_parallel() {
        // ARRANGE
        let first_layer = layer_archive(&[("etc/first", b"first\n")]);
        let second_layer = layer_archive(&[("etc/second", b"second\n")]);
        let first_digest = sha256_digest(&first_layer);
        let second_digest = sha256_digest(&second_layer);
        let delay = Duration::from_millis(500);
        let registry = TestRegistry::start(HashMap::from([
            (
                ("GET".to_string(), format!("/v2/repo/blobs/{first_digest}")),
                HttpResponse::blob(first_layer).with_delay(delay),
            ),
            (
                ("GET".to_string(), format!("/v2/repo/blobs/{second_digest}")),
                HttpResponse::blob(second_layer).with_delay(delay),
            ),
        ]));
        let client = build_client().expect("build HTTP client");
        let output = TempDir::new().expect("create temp dir");
        let layers = vec![
            descriptor(
                &first_digest,
                Some("application/vnd.oci.image.layer.v1.tar+gzip"),
            ),
            descriptor(
                &second_digest,
                Some("application/vnd.oci.image.layer.v1.tar+gzip"),
            ),
        ];

        // ACT
        let started_at = Instant::now();
        download_and_extract_layers(
            &client,
            &image_reference(&registry),
            &layers,
            None,
            output.path(),
        )
        .await
        .expect("download and extract layers");

        // ASSERT
        assert!(started_at.elapsed() < Duration::from_millis(900));
        assert_eq!(
            std::fs::read_to_string(output.path().join("etc/first")).expect("read first file"),
            "first\n"
        );
        assert_eq!(
            std::fs::read_to_string(output.path().join("etc/second")).expect("read second file"),
            "second\n"
        );
    }

    #[tokio::test]
    async fn download_and_extract_layers_rejects_unsupported_layer_media_type() {
        // ARRANGE
        let layer = b"not used".to_vec();
        let digest = sha256_digest(&layer);
        let registry = TestRegistry::start(HashMap::from([(
            ("GET".to_string(), format!("/v2/repo/blobs/{digest}")),
            HttpResponse::blob(layer),
        )]));
        let client = build_client().expect("build HTTP client");
        let output = TempDir::new().expect("create temp dir");
        let layers = vec![descriptor(&digest, Some("application/test"))];

        // ACT
        let error = download_and_extract_layers(
            &client,
            &image_reference(&registry),
            &layers,
            None,
            output.path(),
        )
        .await
        .expect_err("download should fail");

        // ASSERT
        assert!(matches!(error, KociError::UnsupportedLayerMediaType(_)));
    }

    #[tokio::test]
    async fn download_and_extract_layers_allows_empty_layer_list() {
        // ARRANGE
        let registry = TestRegistry::start(HashMap::new());
        let client = build_client().expect("build HTTP client");
        let output = TempDir::new().expect("create temp dir");

        // ACT
        download_and_extract_layers(
            &client,
            &image_reference(&registry),
            &[],
            None,
            output.path(),
        )
        .await
        .expect("download should succeed");

        // ASSERT
        assert!(
            std::fs::read_dir(output.path())
                .expect("read output dir")
                .next()
                .is_none()
        );
    }

    #[tokio::test]
    async fn pull_to_dir_rejects_non_utf8_manifest_response() {
        // ARRANGE
        let registry = TestRegistry::start(HashMap::from([(
            ("GET".to_string(), "/v2/repo/manifests/test".to_string()),
            HttpResponse::manifest(vec![0xff, 0xfe, 0xfd]),
        )]));
        let output = TempDir::new().expect("create temp dir");

        // ACT
        let error = pull_to_dir(
            &registry.reference("repo", "test"),
            "amd64",
            output.path(),
            None,
        )
        .await
        .expect_err("pull should fail");

        // ASSERT
        assert!(matches!(error, KociError::NetworkError(_)));
    }

    #[tokio::test]
    async fn pull_to_dir_rejects_invalid_manifest_json() {
        // ARRANGE
        let registry = TestRegistry::start(HashMap::from([(
            ("GET".to_string(), "/v2/repo/manifests/test".to_string()),
            HttpResponse::manifest(b"not json".to_vec()),
        )]));
        let output = TempDir::new().expect("create temp dir");

        // ACT
        let error = pull_to_dir(
            &registry.reference("repo", "test"),
            "amd64",
            output.path(),
            None,
        )
        .await
        .expect_err("pull should fail");

        // ASSERT
        assert!(matches!(error, KociError::OciParseError(_)));
    }

    #[tokio::test]
    async fn pull_to_dir_propagates_layer_media_type_errors() {
        // ARRANGE
        let layer = b"plain bytes".to_vec();
        let layer_digest = sha256_digest(&layer);
        let manifest = manifest_json(&layer_digest, layer.len(), "application/test");
        let registry = TestRegistry::start(HashMap::from([
            (
                ("GET".to_string(), "/v2/repo/manifests/test".to_string()),
                HttpResponse::manifest(manifest),
            ),
            (
                ("GET".to_string(), format!("/v2/repo/blobs/{layer_digest}")),
                HttpResponse::blob(layer),
            ),
        ]));
        let output = TempDir::new().expect("create temp dir");

        // ACT
        let error = pull_to_dir(
            &registry.reference("repo", "test"),
            "amd64",
            output.path(),
            None,
        )
        .await
        .expect_err("pull should fail");

        // ASSERT
        assert!(matches!(error, KociError::UnsupportedLayerMediaType(_)));
    }
}
