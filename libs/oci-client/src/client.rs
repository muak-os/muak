//! Authenticated registry client shared by all registry-touching operations.

use oci::reference::Image;

use crate::auth::{Access, Credentials, authenticate};
use crate::endpoint;
use crate::error::Result;
use crate::transport::{Transport, for_host};

/// A registry client for one image's registry, authenticated only after a probe.
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
        Self::local(reference)
            .authenticate_with(access, credentials)
            .await
    }

    /// Build a non-authenticating client for cache-only operation.
    ///
    /// Parses `reference` and builds the transport for its registry host, but
    /// leaves the authorization header unset. It performs no authentication
    /// probe and no connection, so it never touches the network while being
    /// constructed.
    #[must_use]
    pub fn local(reference: &str) -> Self {
        let image = Image::parse(reference);
        let http = for_host(&image.registry);

        Self {
            http,
            image,
            authorization: None,
        }
    }

    /// Resolve registry auth from the environment for this client's image.
    ///
    /// Returns a new client carrying the authorization header. The receiver
    /// stays unauthenticated.
    ///
    /// # Errors
    ///
    /// Returns an error when registry authentication fails.
    pub async fn authenticate(&self, access: Access) -> Result<Self> {
        self.authenticate_with(access, None).await
    }

    async fn authenticate_with(
        &self,
        access: Access,
        credentials: Option<Credentials>,
    ) -> Result<Self> {
        let credentials = credentials.or_else(Credentials::from_env);
        let authorization = authenticate(
            &self.http,
            endpoint::scheme(&self.image),
            &self.image.registry,
            &self.image.name,
            access,
            credentials.as_ref(),
        )
        .await?;

        Ok(Self {
            http: self.http.clone(),
            image: self.image.clone(),
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
