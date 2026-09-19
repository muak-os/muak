//! Pinned-entry shapes of the catalog documents.

use serde::{Deserialize, Serialize};

/// Kernel, stub, or installer entry, identified by its `source`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourcedEntry {
    /// Logical identity profiles select (kernels match `KernelSpec`).
    pub source: String,
    /// Repository path relative to the configured registry prefix.
    pub repository: String,
    /// Payload tag the digest was resolved from; informational.
    pub tag: String,
    /// Multi-arch index digest pinning the payload image.
    pub digest: String,
}

/// Overlay or extension entry, identified by its `name`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamedEntry {
    /// Logical identity profiles select.
    pub name: String,
    /// Logical identity of the payload repository.
    pub source: String,
    /// Repository path relative to the configured registry prefix.
    pub repository: String,
    /// Payload tag the digest was resolved from; informational.
    pub tag: String,
    /// Multi-arch index digest pinning the payload image.
    pub digest: String,
}
