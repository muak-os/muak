//! Authenticated registry session shared by all registry-touching operations.

use crate::error::Result;
use crate::image::ImageReference;
use crate::pull::cache::Store;
use crate::registry::auth::{Access, Credentials, authenticate};
use crate::registry::http::{HttpClient, build_client};

/// An authenticated connection to one image's registry.
///
/// Bundles everything every registry operation needs: the parsed image
/// reference, a shared HTTP client, the resolved authorization header value,
/// and the local blob cache.
pub(crate) struct Session {
    /// Local blob and tag-manifest cache.
    pub(crate) cache: Store,
    /// Shared HTTP/HTTPS client.
    pub(crate) client: HttpClient,
    /// Parsed image reference.
    pub(crate) image: ImageReference,
    /// `Authorization` header value for registry requests, when authenticated.
    authorization: Option<String>,
}

impl Session {
    /// Parse the reference, build the client, and resolve registry auth.
    ///
    /// Explicit `credentials` win over the `KOCI_REGISTRY_USERNAME` and
    /// `KOCI_REGISTRY_PASSWORD` environment variables.
    ///
    /// # Errors
    ///
    /// Returns an error when registry authentication fails.
    pub(crate) async fn new(
        reference: &str,
        access: Access,
        credentials: Option<Credentials>,
    ) -> Result<Self> {
        let image = ImageReference::parse(reference);
        let client = build_client();
        let credentials = credentials.or_else(Credentials::from_env);
        let authorization = authenticate(
            &client,
            image.scheme(),
            &image.registry,
            &image.name,
            access,
            credentials.as_ref(),
        )
        .await?;

        Ok(Self {
            cache: Store::new(),
            client,
            image,
            authorization,
        })
    }

    /// `Authorization` header value for registry requests, when authenticated.
    pub(crate) fn authorization(&self) -> Option<&str> {
        self.authorization.as_deref()
    }
}
