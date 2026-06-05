//! Public source resolution API.

use crate::error::Result;
use crate::profile::Profile;
use crate::request::Resolve;
use crate::source;

/// Resolves a profile and request into versioned OCI references.
///
/// # Errors
///
/// Returns an error when the profile references an unknown source input.
pub fn profile(
    request: &Resolve,
    profile: &Profile,
    sources: &source::Sources,
) -> Result<source::ResolvedBuildProfile> {
    source::resolve(request, profile, sources)
}
