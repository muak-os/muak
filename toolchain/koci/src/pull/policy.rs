//! Cache-only versus registry access policy for pulls.

/// Where pull data may be read from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Policy {
    /// Cache first, registry on a miss.
    #[default]
    Network,
    /// Registry for tag references, cache for everything else.
    Refresh,
    /// Cache only; any miss is an error.
    CacheOnly,
}
