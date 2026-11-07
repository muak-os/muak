//! Cache-only versus registry access policy for pulls.

/// Where pull data may be read from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Policy {
    /// Cache first, registry on a miss.
    #[default]
    Network,
    /// Registry for tag references, cache for everything else.
    ///
    /// A tag manifest is re-fetched from the registry and overwrites the
    /// cached tag reference. `sha256:` references and all blobs stay
    /// cache-first.
    Refresh,
    /// Cache only; any miss is an error.
    CacheOnly,
}
