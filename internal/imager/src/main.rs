use backhand::{FilesystemCompressor, FilesystemWriter, NodeHeader, compression::Compressor};
use clap::{Parser, Subcommand};
use cpio::{NewcBuilder, newc::ModeFileType};
use flate2::read::GzDecoder;
use serde::Deserialize;
use std::fs::File;
use std::io::{BufReader, Cursor, Read, Seek, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use tar::Archive;
use walkdir::WalkDir;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Parser)]
#[command(about = "OCI image and initramfs tools")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Build initramfs with OCI extensions
    Build {
        /// Base initramfs image
        #[arg(short, long)]
        base: PathBuf,

        /// Extension sources (local OCI layout path or remote registry reference)
        #[arg(short, long)]
        extension: Vec<String>,

        /// Output initramfs
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Pull OCI image and extract contents
    Pull {
        /// OCI image reference (e.g., ghcr.io/sawangg/muak/installer:latest)
        image: String,

        /// Output directory to extract image contents
        #[arg(short, long)]
        output: PathBuf,
    },
}

// OCI Index (index.json) - also used for manifest lists
#[derive(Deserialize)]
struct OciIndex {
    manifests: Vec<OciDescriptor>,
}

// OCI Manifest
#[derive(Deserialize)]
struct OciManifest {
    #[serde(default)]
    layers: Vec<OciDescriptor>,
    #[serde(default)]
    manifests: Vec<OciDescriptor>,
}

#[derive(Deserialize)]
struct OciDescriptor {
    digest: String,
    #[serde(rename = "mediaType")]
    #[allow(dead_code)]
    media_type: Option<String>,
    #[serde(default)]
    platform: Option<Platform>,
}

#[derive(Deserialize, Default)]
struct Platform {
    architecture: Option<String>,
    os: Option<String>,
}

fn main() -> Result<()> {
    let args = Args::parse();

    match args.command {
        Command::Build {
            base,
            extension,
            output,
        } => cmd_build(&base, &extension, &output),
        Command::Pull { image, output } => cmd_pull(&image, &output),
    }
}

fn cmd_build(base: &Path, extensions: &[String], output: &Path) -> Result<()> {
    // Copy base initramfs to output
    std::fs::copy(base, output)?;

    if extensions.is_empty() {
        println!("No extensions specified, using base initramfs");
        return Ok(());
    }

    // Process each extension
    let mut squashfs_files = Vec::new();
    for ext in extensions {
        let (name, temp_dir) = if Path::new(ext).exists() {
            println!("Processing local extension: {}", ext);
            let name = Path::new(ext)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string();
            let dir = extract_oci_layout(Path::new(ext))?;
            (name, dir)
        } else {
            println!("Pulling remote extension: {}", ext);
            let name = parse_image_name(ext);
            let dir = pull_image_to_temp(ext)?;
            (name, dir)
        };

        println!("Creating squashfs for: {}", name);
        let sqsh_data = create_squashfs(&temp_dir)?;
        squashfs_files.push((format!("extensions/{}.sqsh", name), sqsh_data));
    }

    // Create CPIO archive with extension squashfs files
    println!(
        "Creating CPIO archive with {} extensions",
        squashfs_files.len()
    );
    let cpio_data = create_cpio(&squashfs_files)?;

    // Compress with zstd and append to output
    println!("Compressing and appending to initramfs");
    let compressed = zstd::encode_all(&cpio_data[..], 19)?;
    let mut output_file = std::fs::OpenOptions::new().append(true).open(output)?;
    output_file.write_all(&compressed)?;

    println!(
        "Successfully created initramfs at {} ({} bytes)",
        output.display(),
        std::fs::metadata(output)?.len()
    );

    Ok(())
}

fn cmd_pull(image: &str, output: &Path) -> Result<()> {
    println!("Pulling image: {}", image);

    std::fs::create_dir_all(output)?;
    pull_image_to_dir(image, output)?;

    println!("Successfully extracted to {}", output.display());
    Ok(())
}

/// Extract OCI layout directory, returns temp dir with extracted rootfs
fn extract_oci_layout(oci_dir: &Path) -> Result<PathBuf> {
    let temp = tempfile::tempdir()?;

    // Read index.json
    let index_path = oci_dir.join("index.json");
    let index: OciIndex = serde_json::from_reader(BufReader::new(File::open(&index_path)?))?;

    // Get first manifest (assume single-platform image)
    let manifest_digest = &index.manifests[0].digest;
    let manifest_blob = digest_to_blob_path(oci_dir, manifest_digest);
    let manifest: OciManifest =
        serde_json::from_reader(BufReader::new(File::open(&manifest_blob)?))?;

    // Extract each layer
    for layer in &manifest.layers {
        let layer_path = digest_to_blob_path(oci_dir, &layer.digest);
        extract_layer(&layer_path, temp.path())?;
    }

    Ok(temp.keep())
}

/// Convert digest like "sha256:abc123..." to blob path
fn digest_to_blob_path(oci_dir: &Path, digest: &str) -> PathBuf {
    let hash = digest.strip_prefix("sha256:").unwrap_or(digest);
    oci_dir.join("blobs").join("sha256").join(hash)
}

/// Extract a single layer (tar.gz or raw tar) to destination
fn extract_layer(layer_path: &Path, dest: &Path) -> Result<()> {
    let file = File::open(layer_path)?;
    let mut reader = BufReader::new(file);

    // Read first two bytes to detect gzip magic
    let mut magic = [0u8; 2];
    reader.read_exact(&mut magic)?;
    reader.seek(std::io::SeekFrom::Start(0))?;

    if magic == [0x1f, 0x8b] {
        // Gzip compressed
        let decoder = GzDecoder::new(reader);
        let mut archive = Archive::new(decoder);
        archive.unpack(dest)?;
    } else {
        // Raw tar
        let mut archive = Archive::new(reader);
        archive.unpack(dest)?;
    }

    Ok(())
}

/// Parse image name from reference like "ghcr.io/sawangg/pkgs/cloud-hypervisor:latest"
fn parse_image_name(reference: &str) -> String {
    // Extract the last path component before the tag
    let without_tag = reference.split(':').next().unwrap_or(reference);
    without_tag
        .split('/')
        .last()
        .unwrap_or("extension")
        .to_string()
}

/// Parse registry reference into (registry, name, tag)
fn parse_reference(reference: &str) -> (String, String, String) {
    let (reference, tag) = match reference.rsplit_once(':') {
        Some((r, t)) if !t.contains('/') => (r, t.to_string()),
        _ => (reference, "latest".to_string()),
    };

    let parts: Vec<&str> = reference.splitn(2, '/').collect();
    if parts.len() == 2 && (parts[0].contains('.') || parts[0].contains(':')) {
        let registry = parts[0];
        // Normalize docker.io to the actual registry hostname
        let registry = if registry == "docker.io" {
            "registry-1.docker.io"
        } else {
            registry
        };
        (registry.to_string(), parts[1].to_string(), tag)
    } else {
        // Default to docker.io for simple names
        (
            "registry-1.docker.io".to_string(),
            reference.to_string(),
            tag,
        )
    }
}

/// Pull image from remote registry to a temp directory, returns the temp dir path
fn pull_image_to_temp(reference: &str) -> Result<PathBuf> {
    let temp = tempfile::tempdir()?;
    pull_image_to_dir(reference, temp.path())?;
    Ok(temp.keep())
}

/// Pull image from remote registry and extract to specified directory
fn pull_image_to_dir(reference: &str, dest: &Path) -> Result<()> {
    let (registry, name, tag) = parse_reference(reference);
    let client = reqwest::blocking::Client::builder()
        .user_agent("muak-imager/0.1")
        .build()?;

    // Get auth token if needed (for ghcr.io and docker.io)
    let token = get_auth_token(&client, &registry, &name)?;

    // Get manifest (could be manifest list or direct manifest)
    let manifest_url = format!("https://{}/v2/{}/manifests/{}", registry, name, tag);
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

    if let Some(ref t) = token {
        request = request.header("Authorization", format!("Bearer {}", t));
    }

    let response = request.send()?;
    if !response.status().is_success() {
        return Err(
            format!(
                "Failed to get manifest: {} - {}",
                response.status(),
                manifest_url
            )
            .into(),
        );
    }

    let manifest_text = response.text()?;
    let manifest: OciManifest = serde_json::from_str(&manifest_text).map_err(|e| {
        format!(
            "Failed to parse manifest: {} - body: {}",
            e,
            &manifest_text[..manifest_text.len().min(500)]
        )
    })?;

    // If this is a manifest list, get the amd64 manifest
    let layers = if !manifest.manifests.is_empty() {
        // Find amd64/linux manifest
        let amd64_manifest = manifest
            .manifests
            .iter()
            .find(|m| {
                m.platform.as_ref().is_some_and(|p| {
                    p.architecture.as_deref() == Some("amd64") && p.os.as_deref() == Some("linux")
                })
            })
            .or_else(|| manifest.manifests.first())
            .ok_or("No suitable manifest found in manifest list")?;

        // Fetch the actual manifest
        let manifest_url = format!(
            "https://{}/v2/{}/manifests/{}",
            registry, name, amd64_manifest.digest
        );
        let mut request = client
            .get(&manifest_url)
            .header("Accept", "application/vnd.oci.image.manifest.v1+json")
            .header(
                "Accept",
                "application/vnd.docker.distribution.manifest.v2+json",
            );

        if let Some(ref t) = token {
            request = request.header("Authorization", format!("Bearer {}", t));
        }

        let response = request.send()?;
        if !response.status().is_success() {
            return Err(format!("Failed to get platform manifest: {}", response.status()).into());
        }

        let platform_manifest: OciManifest = response.json()?;
        platform_manifest.layers
    } else {
        manifest.layers
    };

    // Download and extract each layer
    for layer in &layers {
        let blob_url = format!("https://{}/v2/{}/blobs/{}", registry, name, layer.digest);
        let mut request = client.get(&blob_url);

        if let Some(ref t) = token {
            request = request.header("Authorization", format!("Bearer {}", t));
        }

        let response = request.send()?;
        if !response.status().is_success() {
            return Err(format!("Failed to get blob: {}", response.status()).into());
        }

        let bytes = response.bytes()?;

        // Extract tar.gz layer
        let decoder = GzDecoder::new(&bytes[..]);
        let mut archive = Archive::new(decoder);
        archive.unpack(dest)?;
    }

    Ok(())
}

/// Get authentication token for registry (anonymous token for public images)
fn get_auth_token(
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

/// Create squashfs from directory using backhand
fn create_squashfs(dir: &Path) -> Result<Vec<u8>> {
    let mut writer = FilesystemWriter::default();
    let compressor = FilesystemCompressor::new(Compressor::Zstd, None)?;
    writer.set_compressor(compressor);
    writer.set_block_size(1024 * 1024);

    for entry in WalkDir::new(dir).follow_links(false).sort_by_file_name() {
        let entry = entry?;
        let path = entry.path();
        let rel_path = path.strip_prefix(dir)?;

        if rel_path.as_os_str().is_empty() {
            continue;
        }

        let path_str = format!("/{}", rel_path.display());
        let metadata = entry.metadata()?;

        let (uid, gid, mode, mtime) = (
            metadata.uid(),
            metadata.gid(),
            metadata.mode(),
            metadata.mtime(),
        );

        if metadata.is_dir() {
            let header = NodeHeader::new(mode as u16, uid, gid, mtime as u32);
            writer.push_dir(&path_str, header)?;
        } else if metadata.is_symlink() {
            let link_target = std::fs::read_link(path)?;
            let header = NodeHeader::new(0o777, uid, gid, mtime as u32);
            writer.push_symlink(
                link_target.to_string_lossy().into_owned(),
                &path_str,
                header,
            )?;
        } else if metadata.is_file() {
            let contents = std::fs::read(path)?;
            let header = NodeHeader::new(mode as u16, uid, gid, mtime as u32);
            writer.push_file(Cursor::new(contents), &path_str, header)?;
        }
    }

    let mut output = Cursor::new(Vec::new());
    writer.write(&mut output)?;
    Ok(output.into_inner())
}

/// Create CPIO archive from list of (archive_path, data) pairs
fn create_cpio(files: &[(String, Vec<u8>)]) -> Result<Vec<u8>> {
    let mut cpio_data = Vec::new();
    let mut inode = 1u32;

    // Create "extensions" directory entry
    let dir_builder = NewcBuilder::new("extensions")
        .ino(inode)
        .uid(0)
        .gid(0)
        .mode(0o755)
        .set_mode_file_type(ModeFileType::Directory);
    inode += 1;
    let writer = dir_builder.write(&mut cpio_data, 0);
    writer.finish()?;

    // Add each squashfs file
    for (path, data) in files {
        let builder = NewcBuilder::new(path)
            .ino(inode)
            .uid(0)
            .gid(0)
            .mode(0o644)
            .set_mode_file_type(ModeFileType::Regular);
        inode += 1;

        let mut writer = builder.write(&mut cpio_data, data.len() as u32);
        writer.write_all(data)?;
        writer.finish()?;
    }

    // Write CPIO trailer
    cpio::newc::trailer(&mut cpio_data)?;

    Ok(cpio_data)
}
