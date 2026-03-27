//! OCI registry authentication token fetching.

use serde::Deserialize;

use crate::error::{ImagerError, Result};
use crate::oci::http::{HttpClient, collect_body, get};

/// JSON response returned by OCI token endpoints.
#[derive(Deserialize)]
pub(crate) struct TokenResponse {
    pub token: String,
}

/// Return the token endpoint URL for known public registries, or `None` for private ones.
pub(crate) fn get_token_url(registry: &str, name: &str) -> Option<String> {
    if registry == "ghcr.io" {
        Some(format!(
            "https://ghcr.io/token?scope=repository:{}:pull",
            name
        ))
    } else if registry.contains("docker.io") {
        Some(format!(
            "https://auth.docker.io/token?service=registry.docker.io&scope=repository:{}:pull",
            name
        ))
    } else {
        None
    }
}

/// Fetch a Bearer token for the given registry and image name.
pub(crate) async fn fetch_auth_token(
    client: &HttpClient,
    registry: &str,
    name: &str,
) -> Result<Option<String>> {
    let token_url = match get_token_url(registry, name) {
        Some(url) => url,
        None => return Ok(None),
    };

    let resp = match get(client, &token_url, None, &[]).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Warning: Failed to get auth token: {}", e);
            return Ok(None);
        }
    };

    let body = collect_body(resp).await?;
    let text = std::str::from_utf8(&body).map_err(|e| {
        ImagerError::NetworkError(format!("Auth token response is not UTF-8: {}", e))
    })?;
    let token_resp: TokenResponse = serde_json::from_str(text)?;
    Ok(Some(token_resp.token))
}
