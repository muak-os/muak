use flate2::read::GzDecoder;
use serde::Deserialize;
use std::fs::File;
use std::io::{BufReader, Read, Seek};
use std::path::{Path, PathBuf};
use tar::Archive;
use tempfile::TempDir;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Deserialize)]
pub struct OciIndex {
    pub manifests: Vec<OciDescriptor>,
}

#[derive(Deserialize)]
pub struct OciManifest {
    #[serde(default)]
    pub layers: Vec<OciDescriptor>,
    #[serde(default)]
    pub manifests: Vec<OciDescriptor>,
}

#[derive(Deserialize)]
pub struct OciDescriptor {
    pub digest: String,
    #[serde(default)]
    pub platform: Option<Platform>,
}

#[derive(Deserialize, Default)]
pub struct Platform {
    pub architecture: Option<String>,
    pub os: Option<String>,
}

pub struct ImageReference {
    pub registry: String,
    pub name: String,
    pub tag: String,
}

impl ImageReference {
    pub fn parse(reference: &str) -> Self {
        let (reference, tag) = match reference.rsplit_once(':') {
            Some((r, t)) if !t.contains('/') => (r, t.to_string()),
            _ => (reference, "latest".to_string()),
        };

        let parts: Vec<&str> = reference.splitn(2, '/').collect();
        if parts.len() == 2 && (parts[0].contains('.') || parts[0].contains(':')) {
            let registry = if parts[0] == "docker.io" {
                "registry-1.docker.io"
            } else {
                parts[0]
            };
            Self {
                registry: registry.to_string(),
                name: parts[1].to_string(),
                tag,
            }
        } else {
            Self {
                registry: "registry-1.docker.io".to_string(),
                name: reference.to_string(),
                tag,
            }
        }
    }

    pub fn image_name(&self) -> String {
        self.name
            .split('/')
            .next_back()
            .unwrap_or("extension")
            .to_string()
    }
}

pub fn extract_local_oci_layout(oci_dir: &Path) -> Result<PathBuf> {
    let temp = create_temp_dir("oci-")?;

    let index_path = oci_dir.join("index.json");
    let index: OciIndex = serde_json::from_reader(BufReader::new(File::open(&index_path)?))?;

    let manifest_digest = &index.manifests[0].digest;
    let manifest_blob = digest_to_blob_path(oci_dir, manifest_digest);
    let manifest: OciManifest =
        serde_json::from_reader(BufReader::new(File::open(&manifest_blob)?))?;

    for layer in &manifest.layers {
        let layer_path = digest_to_blob_path(oci_dir, &layer.digest);
        extract_tar_layer(&layer_path, temp.path())?;
    }

    Ok(temp.keep())
}

fn digest_to_blob_path(oci_dir: &Path, digest: &str) -> PathBuf {
    let hash = digest.strip_prefix("sha256:").unwrap_or(digest);
    oci_dir.join("blobs").join("sha256").join(hash)
}

fn extract_tar_layer(layer_path: &Path, dest: &Path) -> Result<()> {
    let file = File::open(layer_path)?;
    let mut reader = BufReader::new(file);

    let mut magic = [0u8; 2];
    reader.read_exact(&mut magic)?;
    reader.seek(std::io::SeekFrom::Start(0))?;

    if magic == [0x1f, 0x8b] {
        let decoder = GzDecoder::new(reader);
        let mut archive = Archive::new(decoder);
        archive.unpack(dest)?;
    } else {
        let mut archive = Archive::new(reader);
        archive.unpack(dest)?;
    }

    Ok(())
}

pub fn pull_to_temp(reference: &str) -> Result<PathBuf> {
    let temp = create_temp_dir("oci-")?;
    pull_to_directory(reference, temp.path())?;
    Ok(temp.keep())
}

pub fn pull_to_directory(reference: &str, dest: &Path) -> Result<()> {
    let image_ref = ImageReference::parse(reference);
    let client = reqwest::blocking::Client::builder()
        .user_agent("muak-imager/0.1")
        .build()?;

    let token = fetch_auth_token(&client, &image_ref.registry, &image_ref.name)?;
    let layers = fetch_manifest_layers(&client, &image_ref, token.as_deref())?;

    for layer in &layers {
        download_and_extract_layer(&client, &image_ref, &layer.digest, token.as_deref(), dest)?;
    }

    Ok(())
}

fn fetch_auth_token(
    client: &reqwest::blocking::Client,
    registry: &str,
    name: &str,
) -> Result<Option<String>> {
    #[derive(Deserialize)]
    struct TokenResponse {
        token: String,
    }

    let token_url = if registry == "ghcr.io" {
        format!("https://ghcr.io/token?scope=repository:{}:pull", name)
    } else if registry.contains("docker.io") {
        format!(
            "https://auth.docker.io/token?service=registry.docker.io&scope=repository:{}:pull",
            name
        )
    } else {
        return Ok(None);
    };

    let response = client.get(&token_url).send()?;
    if response.status().is_success() {
        let text = response.text()?;
        let token_resp: TokenResponse = serde_json::from_str(&text).map_err(|e| {
            format!(
                "Failed to parse token response: {} - body: {}",
                e,
                &text[..text.len().min(200)]
            )
        })?;
        Ok(Some(token_resp.token))
    } else {
        eprintln!("Warning: Failed to get auth token: {}", response.status());
        Ok(None)
    }
}

fn fetch_manifest_layers(
    client: &reqwest::blocking::Client,
    image_ref: &ImageReference,
    token: Option<&str>,
) -> Result<Vec<OciDescriptor>> {
    let manifest_url = format!(
        "https://{}/v2/{}/manifests/{}",
        image_ref.registry, image_ref.name, image_ref.tag
    );

    let mut request = client
        .get(&manifest_url)
        .header("Accept", "application/vnd.oci.image.manifest.v1+json")
        .header(
            "Accept",
            "application/vnd.docker.distribution.manifest.v2+json",
        )
        .header("Accept", "application/vnd.oci.image.index.v1+json")
        .header(
            "Accept",
            "application/vnd.docker.distribution.manifest.list.v2+json",
        );

    if let Some(t) = token {
        request = request.header("Authorization", format!("Bearer {}", t));
    }

    let response = request.send()?;
    if !response.status().is_success() {
        return Err(format!(
            "Failed to get manifest: {} - {}",
            response.status(),
            manifest_url
        )
        .into());
    }

    let manifest_text = response.text()?;
    let manifest: OciManifest = serde_json::from_str(&manifest_text).map_err(|e| {
        format!(
            "Failed to parse manifest: {} - body: {}",
            e,
            &manifest_text[..manifest_text.len().min(500)]
        )
    })?;

    if !manifest.manifests.is_empty() {
        fetch_platform_manifest_layers(client, image_ref, &manifest.manifests, token)
    } else {
        Ok(manifest.layers)
    }
}

fn fetch_platform_manifest_layers(
    client: &reqwest::blocking::Client,
    image_ref: &ImageReference,
    manifests: &[OciDescriptor],
    token: Option<&str>,
) -> Result<Vec<OciDescriptor>> {
    let amd64_manifest = manifests
        .iter()
        .find(|m| {
            m.platform.as_ref().is_some_and(|p| {
                p.architecture.as_deref() == Some("amd64") && p.os.as_deref() == Some("linux")
            })
        })
        .or_else(|| manifests.first())
        .ok_or("No suitable manifest found in manifest list")?;

    let manifest_url = format!(
        "https://{}/v2/{}/manifests/{}",
        image_ref.registry, image_ref.name, amd64_manifest.digest
    );

    let mut request = client
        .get(&manifest_url)
        .header("Accept", "application/vnd.oci.image.manifest.v1+json")
        .header(
            "Accept",
            "application/vnd.docker.distribution.manifest.v2+json",
        );

    if let Some(t) = token {
        request = request.header("Authorization", format!("Bearer {}", t));
    }

    let response = request.send()?;
    if !response.status().is_success() {
        return Err(format!("Failed to get platform manifest: {}", response.status()).into());
    }

    let platform_manifest: OciManifest = response.json()?;
    Ok(platform_manifest.layers)
}

fn download_and_extract_layer(
    client: &reqwest::blocking::Client,
    image_ref: &ImageReference,
    digest: &str,
    token: Option<&str>,
    dest: &Path,
) -> Result<()> {
    let blob_url = format!(
        "https://{}/v2/{}/blobs/{}",
        image_ref.registry, image_ref.name, digest
    );

    let mut request = client.get(&blob_url);
    if let Some(t) = token {
        request = request.header("Authorization", format!("Bearer {}", t));
    }

    let response = request.send()?;
    if !response.status().is_success() {
        return Err(format!("Failed to get blob: {}", response.status()).into());
    }

    let bytes = response.bytes()?;
    let decoder = GzDecoder::new(&bytes[..]);
    let mut archive = Archive::new(decoder);
    archive.unpack(dest)?;

    Ok(())
}

fn create_temp_dir(prefix: &str) -> Result<TempDir> {
    const TEMP_DIRS: &[&str] = &["/run/install", "/run/state/update", "/run", "/tmp"];

    for dir in TEMP_DIRS {
        let path = Path::new(dir);
        if path.exists()
            && let Ok(temp) = tempfile::Builder::new().prefix(prefix).tempdir_in(path)
        {
            return Ok(temp);
        }
    }

    Err("Failed to create temp directory in any location".into())
}
