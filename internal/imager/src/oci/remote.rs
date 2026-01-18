use reqwest::blocking::Client;
use std::path::{Path, PathBuf};

use crate::error::{ImagerError, Result};
use crate::image::ImageReference;
use crate::oci::USER_AGENT;
use crate::oci::auth::fetch_auth_token;
use crate::oci::layer;
use crate::oci::manifest;

pub(crate) fn pull_to_temp(reference: &str) -> Result<PathBuf> {
    let temp = create_temp_dir("oci-")?;
    pull_to_dir(reference, temp.path())?;
    Ok(temp.keep())
}

pub(crate) fn pull_to_dir(reference: &str, dest: &Path) -> Result<()> {
    let image_ref = ImageReference::parse(reference);
    let client = Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| ImagerError::NetworkError(format!("Failed to create HTTP client: {}", e)))?;

    let token = fetch_auth_token(&client, &image_ref.registry, &image_ref.name)?;
    let manifest_url = manifest::build_url(&image_ref, &image_ref.tag);
    let manifest_json = manifest::fetch(&client, &manifest_url, token.as_deref())?;
    let manifest = manifest::parse(&manifest_json)?;

    let layers = if !manifest.manifests.is_empty() {
        let selected_manifest = manifest::select_platform(&manifest.manifests)?;
        let platform_url = manifest::build_url(&image_ref, &selected_manifest.digest);
        let platform_json = manifest::fetch(&client, &platform_url, token.as_deref())?;
        let platform_manifest = manifest::parse(&platform_json)?;
        platform_manifest.layers
    } else {
        manifest.layers
    };

    for layer in &layers {
        let bytes = layer::download_blob(&client, &image_ref, &layer.digest, token.as_deref())?;
        layer::extract_archive(&bytes, dest)?;
    }

    Ok(())
}

pub(crate) fn create_temp_dir(prefix: &str) -> Result<tempfile::TempDir> {
    let locations = ["/run", "/tmp"];
    for &dir in &locations {
        if let Ok(temp) = tempfile::Builder::new().prefix(prefix).tempdir_in(dir) {
            return Ok(temp);
        }
    }
    Err(ImagerError::TempDirError(format!(
        "Failed to create temp dir in any of: {:?}",
        locations
    )))
}
