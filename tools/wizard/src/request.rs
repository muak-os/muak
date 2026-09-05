//! User request for image building.

use core::fmt;
use std::io::Write;

use koci::arch::Arch;
use sbolt::keys::SigningPair;
use serde::{Deserialize, Serialize};

use crate::artifact::Artifact;
use crate::domain::overlay;
use crate::domain::profile::Profile;
use crate::domain::resolution::Resolution;
use crate::error::{Result, WizardError};
use crate::nodes::entry_sizes;
use crate::pipeline::context::{BuildContext, TargetWriters};
use crate::pipeline::execute::execute;
use crate::pipeline::plan::plan;
use crate::resolver;

/// A build request expressing what to build and where to write each artifact.
pub struct Request<'a> {
    version: String,
    platform: Platform,
    arch: Option<Arch>,
    signing: Option<&'a SigningPair<'a>>,
    targets: Vec<(Artifact, &'a mut (dyn Write + Send))>,
}

impl fmt::Debug for Request<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Request")
            .field("version", &self.version)
            .field("platform", &self.platform)
            .field("arch", &self.arch)
            .field("signing", &self.signing.is_some())
            .field(
                "targets",
                &self.targets.iter().map(|item| &item.0).collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl<'a> Request<'a> {
    /// Creates a new request for the given version and platform.
    #[must_use]
    pub fn new<V: Into<String>>(version: V, platform: Platform) -> Self {
        Self {
            version: version.into(),
            platform,
            arch: None,
            signing: None,
            targets: Vec::new(),
        }
    }

    /// Sets the target CPU architecture (`None` defaults to host arch).
    #[must_use]
    pub fn arch(mut self, arch: Arch) -> Self {
        self.arch = Some(arch);

        self
    }

    /// Sets the output writer for an artifact kind.
    ///
    /// # Errors
    ///
    /// Returns an error when a target for the same artifact kind was already set.
    pub fn artifact(mut self, kind: Artifact, writer: &'a mut (dyn Write + Send)) -> Result<Self> {
        if self.targets.iter().any(|item| item.0 == kind) {
            return Err(WizardError::BuildError(format!(
                "duplicate artifact target: {kind}"
            )));
        }
        self.targets.push((kind, writer));

        Ok(self)
    }

    /// Returns the requested version.
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Returns the target deployment platform.
    #[must_use]
    pub const fn platform(&self) -> Platform {
        self.platform
    }

    /// Returns the target CPU architecture (`None` when host arch should be used).
    #[must_use]
    pub const fn target_arch(&self) -> Option<Arch> {
        self.arch
    }

    /// Sets the optional signing key for Authenticode PE signing of the UKI.
    #[must_use]
    pub fn sign(mut self, key: &'a SigningPair<'a>) -> Self {
        self.signing = Some(key);

        self
    }

    /// Returns the artifact kinds targeted by this request.
    pub fn targets(&self) -> impl Iterator<Item = &Artifact> {
        self.targets.iter().map(|item| &item.0)
    }

    /// Resolves and builds all requested artifacts.
    ///
    /// # Errors
    ///
    /// Returns an error when resolution, pulling, building, or signing fails.
    pub fn build(self, profile: &Profile) -> Result<crate::Metadata> {
        if self.targets.is_empty() {
            return Err(WizardError::BuildError(
                "at least one artifact must be requested".to_owned(),
            ));
        }

        let mut resolution = resolver::plan(&self, profile)?;
        discover_assets(&mut resolution)?;
        let profile_bytes = profile.canonical_bytes()?;
        let artifacts: Vec<Artifact> = self.targets.iter().map(|target| target.0).collect();
        let ctx = BuildContext {
            build: resolution.build(),
            profile: &profile_bytes,
            signing: self.signing,
        };
        let mut writers = TargetWriters::new(self.targets);
        let graph = plan(&ctx, &artifacts)?;

        execute(graph, &ctx, &mut writers)
    }
}

/// Deployment platform; determines boot and disk behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    /// Bare-metal installation.
    Metal,
    /// Amazon Web Services.
    Aws,
    /// Google Cloud Platform.
    Gcp,
}

impl Platform {
    /// Returns the lowercase path segment for this platform.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Metal => "metal",
            Self::Aws => "aws",
            Self::Gcp => "gcp",
        }
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Discovers every overlay asset from the overlay image's `dev.muak.sizes`
/// annotation and stores them on the resolution.
///
/// # Errors
///
/// Returns an error when the sizes annotation is missing or malformed or an
/// entry references a malformed placement.
fn discover_assets(resolution: &mut Resolution) -> Result<()> {
    let Some(overlay) = resolution.build().overlay() else {
        return Ok(());
    };
    let entries: Vec<(String, u64)> = entry_sizes(&overlay.source, overlay.arch)?
        .into_iter()
        .collect();
    resolution.set_overlay_assets(Some(overlay::classify(overlay, entries)?));

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arch_conversions() {
        // ARRANGE & ACT & ASSERT
        assert_eq!(Arch::Amd64.as_str(), "amd64");
        assert_eq!(Arch::Arm64.as_str(), "arm64");
        assert_eq!(Arch::Riscv64.as_str(), "riscv64");
        assert_eq!(format!("{}", Arch::Amd64), "amd64");
        assert_eq!(format!("{}", Arch::Arm64), "arm64");
        assert_eq!(format!("{}", Arch::Riscv64), "riscv64");
        assert_eq!(format!("{}", Platform::Metal), "metal");
    }

    #[test]
    fn platform_display() {
        // ARRANGE & ACT & ASSERT
        assert_eq!(Platform::Metal.as_str(), "metal");
        assert_eq!(format!("{}", Platform::Aws), "aws");
        assert_eq!(format!("{}", Platform::Gcp), "gcp");
    }

    #[test]
    fn builder_rejects_duplicate_artifact() {
        // ARRANGE
        let mut buf1 = Vec::new();
        let mut buf2 = Vec::new();

        // ACT
        let result = Request::new("v1.0.0", Platform::Metal)
            .artifact(Artifact::Kernel, &mut buf1)
            .expect("first kernel")
            .artifact(Artifact::Kernel, &mut buf2);

        // ASSERT
        result.unwrap_err();
    }

    #[test]
    fn builder_chains_multiple_artifacts() {
        // ARRANGE
        let mut kernel_buf = Vec::new();
        let mut iso_buf = Vec::new();

        // ACT
        let request = Request::new("v1.0.0", Platform::Metal)
            .arch(Arch::Amd64)
            .artifact(Artifact::Kernel, &mut kernel_buf)
            .expect("kernel")
            .artifact(Artifact::Iso, &mut iso_buf)
            .expect("iso");

        // ASSERT
        assert_eq!(request.targets().count(), 2);
        assert_eq!(request.version(), "v1.0.0");
        assert_eq!(request.platform(), Platform::Metal);
        assert_eq!(request.target_arch(), Some(Arch::Amd64));
    }
}
