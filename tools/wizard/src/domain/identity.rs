//! Content-addressed identity types for profiles and resolutions.

use core::fmt;

use kata::schema::view::{EntryRef, Role};
use sha2::{Digest as _, Sha256};

const PROFILE_API_VERSION: &str = "muak.dev/profile/v1-beta";
const RESOLUTION_API_VERSION: &str = "muak.dev/resolution/v1-beta";

macro_rules! id_type {
    ($name:ident, $doc:literal) => {
        /// Content-addressed identity type.
        #[doc = $doc]
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name([u8; 32]);

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

    /// Returns the raw identity bytes.
    #[must_use]
    pub(crate) const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

id_type!(ResolutionId, "Identity of one exact resolved build.");

impl ResolutionId {
    /// Computes the resolution identity over the frozen recipe.
    #[must_use]
    pub fn compute(
        profile: &ProfileId,
        inputs: &[ResolvedInput],
        arch: &str,
        policy: &str,
    ) -> Self {
        let mut sorted = inputs.to_vec();
        sorted.sort_by(|left, right| {
            (&left.kind, &left.identity).cmp(&(&right.kind, &right.identity))
        });

        let mut context = Sha256::new();
        context.update(RESOLUTION_API_VERSION.as_bytes());
        context.update(b"\0");
        context.update(profile.as_bytes());
        for input in &sorted {
            context.update(input.kind.as_str().as_bytes());
            context.update(b"\0");
            context.update(input.identity.as_bytes());
            context.update(b"\0");
            context.update(input.digest.as_bytes());
            context.update(b"\0");
        }
        context.update(arch.as_bytes());
        context.update(b"\0");
        context.update(policy.as_bytes());
        let mut out = [0_u8; 32];
        out.copy_from_slice(context.finalize().as_ref());

        Self(out)
    }
}

/// One resolved input record of the resolution identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedInput {
    /// Role the input plays in the build.
    pub kind: Role,
    /// Logical identity the profile selected (`source` or `name`).
    pub identity: String,
    /// Multi-arch index digest pinning the input image.
    pub digest: String,
}

impl From<EntryRef<'_>> for ResolvedInput {
    fn from(view: EntryRef<'_>) -> Self {
        Self {
            kind: view.role,
            identity: view.identity.to_owned(),
            digest: view.digest.to_owned(),
        }
    }
}

fn domain_hash(domain: &[u8], data: &[u8]) -> [u8; 32] {
    let mut context = Sha256::new();
    context.update(domain);
    context.update(b"\0");
    context.update(data);
    let mut out = [0_u8; 32];
    out.copy_from_slice(context.finalize().as_ref());

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(kind: Role, identity: &str, digest: &str) -> ResolvedInput {
        ResolvedInput {
            kind,
            identity: identity.to_owned(),
            digest: digest.to_owned(),
        }
    }

    #[test]
    fn ids_are_domain_separated() {
        // ARRANGE
        let data = b"payload";
        let profile = ProfileId::new(data);
        let resolution = ResolutionId::compute(&profile, &[], "amd64", "default");

        // ACT
        let profile_again = ProfileId::new(data);

        // ASSERT
        assert_eq!(profile, profile_again);
        assert_ne!(format!("{profile}"), format!("{resolution}"));
        assert_eq!(format!("{profile}").len(), 64);
        assert_eq!(format!("{resolution}").len(), 64);
    }

    #[test]
    fn same_inputs_produce_same_ids() {
        // ARRANGE
        let profile = ProfileId::new(b"data");
        let inputs = [input(Role::Kernel, "muak-os/linux", "sha256:1111")];

        // ACT
        let first = ResolutionId::compute(&profile, &inputs, "amd64", "default");
        let second = ResolutionId::compute(&profile, &inputs, "amd64", "default");

        // ASSERT
        assert_eq!(first, second);
    }

    #[test]
    fn input_order_does_not_affect_identity() {
        // ARRANGE
        let profile = ProfileId::new(b"data");
        let forward = [
            input(Role::Kernel, "muak-os/linux", "sha256:1111"),
            input(Role::Extension, "muak-os/qemu", "sha256:4444"),
        ];
        let reversed = [forward[1].clone(), forward[0].clone()];

        // ACT
        let first = ResolutionId::compute(&profile, &forward, "amd64", "default");
        let second = ResolutionId::compute(&profile, &reversed, "amd64", "default");

        // ASSERT
        assert_eq!(first, second);
    }

    #[test]
    fn identity_varies_with_digest_arch_and_policy() {
        // ARRANGE
        let profile = ProfileId::new(b"data");
        let inputs = [input(Role::Kernel, "muak-os/linux", "sha256:1111")];
        let other_digest = [input(Role::Kernel, "muak-os/linux", "sha256:2222")];

        // ACT
        let base = ResolutionId::compute(&profile, &inputs, "amd64", "default");
        let re_pinned = ResolutionId::compute(&profile, &other_digest, "amd64", "default");
        let other_arch = ResolutionId::compute(&profile, &inputs, "arm64", "default");
        let other_policy = ResolutionId::compute(&profile, &inputs, "amd64", "locked");

        // ASSERT
        assert_ne!(base, re_pinned);
        assert_ne!(base, other_arch);
        assert_ne!(base, other_policy);
    }
}
