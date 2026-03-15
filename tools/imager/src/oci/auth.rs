use reqwest::Client;
use serde::Deserialize;

use crate::error::{ImagerError, Result};

#[derive(Deserialize)]
pub(crate) struct TokenResponse {
    pub token: String,
}

/// Get the token URL for a registry, if supported.
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

pub(crate) async fn fetch_auth_token(
    client: &Client,
    registry: &str,
    name: &str,
) -> Result<Option<String>> {
    let token_url = match get_token_url(registry, name) {
        Some(url) => url,
        None => return Ok(None),
    };

    let response = client
        .get(&token_url)
        .send()
        .await
        .map_err(|e| ImagerError::NetworkError(format!("Failed to fetch auth token: {}", e)))?;
    if !response.status().is_success() {
        eprintln!("Warning: Failed to get auth token: {}", response.status());
        return Ok(None);
    }

    let text = response.text().await.map_err(|e| {
        ImagerError::NetworkError(format!("Failed to read auth token response: {}", e))
    })?;
    let token_resp: TokenResponse = serde_json::from_str(&text)?;
    Ok(Some(token_resp.token))
}
