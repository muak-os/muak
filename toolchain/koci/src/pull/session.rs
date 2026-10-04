//! Pull sessions: one manifest resolution serving many consumers.

use oci::arch::Arch;
use oci::model::Descriptor;
use oci_client::auth::Access;
use oci_client::client::Client;

use super::cache::Store;
use super::content::Content;
use super::entries::FileEntry;
use super::lazy::LazyLayers;
use super::{layer, resolve, scan};
use crate::error::Result;
use crate::progress::Progress;
use crate::registry;
use crate::runtime;
use crate::signature::Verification;

/// An opened pull session over one image reference.
pub struct Session {
    client: Client,
    cache: Store,
    content: Content,
    layers: Vec<Descriptor>,
    reference: String,
    arch: String,
}

/// Open a pull session for `reference`, resolving the platform manifest once.
///
/// # Errors
///
/// Returns an error when the reference cannot be parsed, the registry
/// handshake fails, or signature verification fails.
pub fn open(
    reference: &str,
    arch: &Arch,
    verification: Option<&Verification<'_>>,
) -> Result<Session> {
    runtime::runtime()?.block_on(async {
        let client = registry::connect(reference, Access::Pull).await?;
        let cache = Store::new();
        let layers = resolve::layers(&client, &cache, arch, verification).await?;

        Ok(Session {
            client,
            cache,
            content: Content::new(),
            layers,
            reference: reference.to_owned(),
            arch: arch.as_str().to_owned(),
        })
    })
}

impl Session {
    /// The resolved layer descriptors of the platform manifest, not yet downloaded.
    #[must_use]
    pub fn layers(&self) -> &[Descriptor] {
        &self.layers
    }

    /// Stream every live file entry of the image over a single pull.
    ///
    /// # Errors
    ///
    /// Returns an error when a layer cannot be fetched or decompressed, or the
    /// handler returns an error.
    pub fn walk<F>(&self, progress: &dyn Progress, mut handler: F) -> Result<()>
    where
        F: FnMut(FileEntry<'_>) -> Result<()>,
    {
        progress.pulling(&self.reference, &self.arch);
        progress.resolved(self.layers.len());

        runtime::runtime()?.block_on(layer::walk(
            &self.client,
            &self.cache,
            &self.content,
            &self.layers,
            progress,
            |_layer_idx, entry, info| scan::handle_file_entry(entry, info, &mut handler),
        ))
    }

    /// Lazy layer handles that download each blob on first read.
    #[must_use]
    pub fn lazy(&self) -> LazyLayers<'_> {
        LazyLayers::new(self)
    }

    /// The authenticated registry client of the session.
    pub(crate) fn client(&self) -> &Client {
        &self.client
    }

    /// The cache store of the session.
    pub(crate) fn cache(&self) -> &Store {
        &self.cache
    }

    /// The content store of the session.
    pub(crate) fn content(&self) -> &Content {
        &self.content
    }
}
