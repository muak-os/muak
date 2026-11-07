//! Pull sessions: one manifest resolution serving many consumers.

use oci::arch::Arch;
use oci::model::Descriptor;
use oci_client::client::Client;

use super::cache::Store;
use super::content::Content;
use super::entries::FileEntry;
use super::lazy::LazyLayers;
use super::policy::Policy;
use super::{layer, resolve, scan};
use crate::error::Result;
use crate::progress::{Event, Operation, Reporter};
use crate::runtime;
use crate::signature::Verification;

/// An opened pull session over one image reference.
pub struct Session {
    client: Client,
    content: Content,
    layers: Vec<Descriptor>,
    digest: String,
    reference: String,
    arch: String,
    policy: Policy,
}

impl Session {
    /// The resolved layer descriptors of the platform manifest, not yet downloaded.
    #[must_use]
    pub fn layers(&self) -> &[Descriptor] {
        &self.layers
    }

    /// The resolved platform manifest digest.
    #[must_use]
    pub fn digest(&self) -> &str {
        &self.digest
    }

    /// Stream every live file entry of the image over a single pull.
    ///
    /// # Errors
    ///
    /// Returns an error when a layer cannot be fetched or decompressed, or the
    /// handler returns an error.
    pub fn walk<F>(&self, reporter: &dyn Reporter, mut handler: F) -> Result<()>
    where
        F: FnMut(FileEntry<'_>) -> Result<()>,
    {
        let subject = format!("{} {}", self.reference, self.arch);
        reporter.report(Event::Started {
            operation: Operation::Pull,
            subject: &subject,
        });
        reporter.report(Event::Resolved {
            layers: self.layers.len(),
            digest: &self.digest,
        });

        runtime::runtime()?.block_on(layer::walk(
            &self.client,
            &self.content,
            &self.layers,
            reporter,
            self.policy,
            |_layer_idx, entry, info| scan::handle_file_entry(entry, info, &mut handler),
        ))
    }

    /// Lazy layer handles that download each blob on first read.
    #[must_use]
    pub fn lazy(&self) -> LazyLayers<'_> {
        LazyLayers::new(self)
    }

    /// The registry client of the session, unauthenticated until a miss needs auth.
    pub(crate) fn client(&self) -> &Client {
        &self.client
    }

    /// The content store of the session.
    pub(crate) fn content(&self) -> &Content {
        &self.content
    }

    /// The cache access policy of the session.
    pub(crate) fn policy(&self) -> Policy {
        self.policy
    }
}

/// Open a pull session for `reference` under `policy`.
///
/// # Errors
///
/// Returns an error when the reference cannot be parsed, the registry
/// handshake fails, signature verification fails, or a cache-only miss occurs.
pub fn open(
    reference: &str,
    arch: &Arch,
    verification: Option<&Verification<'_>>,
    policy: Policy,
) -> Result<Session> {
    runtime::runtime()?.block_on(async {
        let client = Client::local(reference);
        let cache = Store::new();
        let content = Content::new();
        let (layers, digest) =
            resolve::layers(&client, &cache, &content, arch, verification, policy).await?;

        Ok(Session {
            client,
            content,
            layers,
            digest,
            reference: reference.to_owned(),
            arch: arch.as_str().to_owned(),
            policy,
        })
    })
}
