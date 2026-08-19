//! Content-addressed identity types for profiles, releases, and resolutions.

use core::fmt;

use ring::digest;

pub(crate) const RELEASE_API_VERSION: &str = "muak-release-v1";
const PROFILE_API_VERSION: &str = "muak-profile-v1";
const RESOLUTION_API_VERSION: &str = "muak-resolution-v1";

macro_rules! id_type {
    ($name:ident, $doc:literal) => {
        /// Content-addressed identity type.
        #[doc = $doc]
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name([u8; 32]);

        impl $name {
            /// Returns the raw identity bytes.
            #[must_use]
            pub const fn as_bytes(&self) -> &[u8; 32] {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                for byte in self.0 {
                    f.write_fmt(format_args!("{byte:02x}"))?;
                }

                Ok(())
            }
        }
    };
}

id_type!(
    ProfileId,
    "Version-neutral identity of a customized profile."
);

impl ProfileId {
    /// Computes the profile identity over canonical spec bytes.
    pub(crate) fn new(data: &[u8]) -> Self {
        Self(domain_hash(PROFILE_API_VERSION.as_bytes(), data))
    }
}

id_type!(ReleaseManifestId, "Content identity of a release manifest.");

impl ReleaseManifestId {
    /// Computes the release manifest identity over canonical manifest bytes.
    pub(crate) fn new(data: &[u8]) -> Self {
        Self(domain_hash(RELEASE_API_VERSION.as_bytes(), data))
    }
}

id_type!(ResolutionId, "Identity of one exact resolved build.");

impl ResolutionId {
    /// Computes the resolution identity over the full resolution context.
    #[must_use]
    pub fn compute(
        profile: &ProfileId,
        release: &ReleaseManifestId,
        arch: &str,
        platform: &str,
        policy: &str,
    ) -> Self {
        let mut context = digest::Context::new(&digest::SHA256);
        context.update(RESOLUTION_API_VERSION.as_bytes());
        context.update(b"\0");
        context.update(profile.as_bytes());
        context.update(release.as_bytes());
        context.update(arch.as_bytes());
        context.update(platform.as_bytes());
        context.update(policy.as_bytes());
        let mut out = [0_u8; 32];
        out.copy_from_slice(context.finish().as_ref());

        Self(out)
    }
}

/// Domain-separated SHA-256 over `data`, with a NUL between domain and data.
fn domain_hash(domain: &[u8], data: &[u8]) -> [u8; 32] {
    let mut context = digest::Context::new(&digest::SHA256);
    context.update(domain);
    context.update(b"\0");
    context.update(data);
    let mut out = [0_u8; 32];
    out.copy_from_slice(context.finish().as_ref());

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_domain_separated() {
        // ARRANGE
        let data = b"payload";
        let profile = ProfileId::new(data);
        let release = ReleaseManifestId::new(data);
        let resolution = ResolutionId::compute(&profile, &release, "amd64", "metal", "default");

        // ACT
        let profile_again = ProfileId::new(data);
        let release_again = ReleaseManifestId::new(data);

        // ASSERT
        assert_eq!(profile, profile_again);
        assert_eq!(release, release_again);
        assert_ne!(profile.as_bytes(), release.as_bytes());
        assert_ne!(profile.as_bytes(), resolution.as_bytes());
        assert_ne!(release.as_bytes(), resolution.as_bytes());
        assert_eq!(format!("{profile}").len(), 64);
        assert_eq!(format!("{release}").len(), 64);
        assert_eq!(format!("{resolution}").len(), 64);
    }

    #[test]
    fn same_inputs_produce_same_ids() {
        // ARRANGE / ACT
        let first = ProfileId::new(b"data");
        let second = ProfileId::new(b"data");

        // ASSERT
        assert_eq!(first, second);
    }

    #[test]
    fn resolution_id_varies_with_arch_platform_and_policy() {
        // ARRANGE
        let profile = ProfileId::new(b"profile");
        let release = ReleaseManifestId::new(b"release");

        // ACT
        let base = ResolutionId::compute(&profile, &release, "amd64", "metal", "default");
        let other_arch = ResolutionId::compute(&profile, &release, "arm64", "metal", "default");
        let other_platform = ResolutionId::compute(&profile, &release, "amd64", "aws", "default");
        let other_policy = ResolutionId::compute(&profile, &release, "amd64", "metal", "locked");

        // ASSERT
        assert_ne!(base, other_arch);
        assert_ne!(base, other_platform);
        assert_ne!(base, other_policy);
    }
}
