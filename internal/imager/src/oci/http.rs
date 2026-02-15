use reqwest::Client;

use crate::error::{ImagerError, Result};

/// Build an authenticated HTTP request with optional token and accept headers.
pub(crate) async fn build_authenticated_request(
    client: &Client,
    url: &str,
    token: Option<&str>,
    accept_headers: &[&str],
) -> Result<reqwest::Response> {
    let mut request = client.get(url);
    for header in accept_headers {
        request = request.header("Accept", *header);
    }
    if let Some(t) = token {
        request = request.header("Authorization", format!("Bearer {}", t));
    }
    let response = request
        .send()
        .await
        .map_err(|e| ImagerError::NetworkError(format!("HTTP request failed: {}", e)))?;
    if !response.status().is_success() {
        return Err(ImagerError::DownloadError(format!(
            "HTTP request failed with status: {} for URL: {}",
            response.status(),
            url
        )));
    }
    Ok(response)
}
