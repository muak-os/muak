//! Authenticated registry client shared by all registry-touching operations.

use oci::reference::Image;

use crate::auth::{Access, Credentials, authenticate};
use crate::error::Result;
use crate::http::{Transport, build_client};

/// An authenticated connection to one image's registry.
#[derive(Clone)]
pub struct Client {
    /// Shared HTTP/HTTPS client.
    http: Transport,
    /// Parsed image reference.
    image: Image,
    /// `Authorization` header value for registry requests, when authenticated.
    authorization: Option<String>,
}

impl Client {
    /// Parse the reference, build the client, and resolve registry auth.
    ///
    /// # Errors
    ///
    /// Returns an error when registry authentication fails.
    pub async fn new(
        reference: &str,
        access: Access,
        credentials: Option<Credentials>,
    ) -> Result<Self> {
        let image = Image::parse(reference);
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
            http: client,
            image,
            authorization,
        })
    }

    /// Shared HTTP/HTTPS client for raw registry requests.
    #[must_use]
    pub fn http(&self) -> &Transport {
        &self.http
    }

    /// Parsed image reference.
    #[must_use]
    pub fn image(&self) -> &Image {
        &self.image
    }

    /// `Authorization` header value for registry requests, when authenticated.
    #[must_use]
    pub fn authorization(&self) -> Option<&str> {
        self.authorization.as_deref()
    }
}
