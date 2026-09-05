//! Authenticated registry session shared by all registry-touching operations.

use crate::error::Result;
use crate::image::ImageReference;
use crate::pull::cache::Store;
use crate::registry::auth::fetch_auth_token;
use crate::registry::http::{HttpClient, build_client};

/// An authenticated connection to one image's registry.
///
/// Bundles everything every registry operation needs: the parsed image
/// reference, a shared HTTP client, an auth token, and the local blob cache.
pub(crate) struct Session {
    /// Local blob and tag-manifest cache.
    pub(crate) cache: Store,
    /// Shared HTTP/HTTPS client.
    pub(crate) client: HttpClient,
    /// Parsed image reference.
    pub(crate) image: ImageReference,
    /// Bearer token, when the registry issues one.
    token: Option<String>,
}

impl Session {
    /// Parse the reference, build the client, and fetch an auth token.
    ///
    /// # Errors
    ///
    /// Returns an error when the auth token request fails.
    pub(crate) async fn new(reference: &str) -> Result<Self> {
        let image = ImageReference::parse(reference);
        let client = build_client();
        let token = fetch_auth_token(&client, &image.registry, &image.name).await?;

        Ok(Self {
            cache: Store::new(),
            client,
            image,
            token,
        })
    }

    /// Bearer token for registry requests.
    pub(crate) fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }
}
