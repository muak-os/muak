//! User request for image building.

use core::fmt;
use std::io::Write;

use koci::arch::Arch;
use sbolt::keys::SigningPair;
use serde::{Deserialize, Serialize};

use crate::artifact::Artifact;
use crate::build;
use crate::error::{Result, WizardError};
use crate::profile::Profile;
use crate::resolve;

/// A build request expressing what to build and where to write each artifact.
pub struct Request<'a> {
    version: String,
    platform: Platform,
    arch: Option<Arch>,
    signing_key: Option<&'a SigningPair<'a>>,
    targets: Vec<(Artifact, &'a mut dyn Write)>,
}

impl fmt::Debug for Request<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Request")
            .field("version", &self.version)
            .field("platform", &self.platform)
            .field("arch", &self.arch)
            .field("signing_key", &self.signing_key.is_some())
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
            signing_key: None,
            targets: Vec::new(),
        }
    }

    /// Sets the target CPU architecture (`None` defaults to host arch).
    #[must_use]
    pub fn arch(mut self, arch: Arch) -> Self {
        self.arch = Some(arch);

        self
    }

    /// Binds an artifact kind to an output writer.
    ///
    /// # Errors
    ///
    /// Returns an error when a target for the same artifact kind was already set.
    pub fn artifact(mut self, kind: Artifact, writer: &'a mut dyn Write) -> Result<Self> {
        if self.targets.iter().any(|item| item.0 == kind) {
            return Err(WizardError::BuildError(format!(
                "duplicate artifact target: {kind}"
            )));
        }
        self.targets.push((kind, writer));

        Ok(self)
    }

    /// Sets the output writer for the kernel image.
    ///
    /// # Errors
    ///
    /// Returns an error when a kernel target was already set.
    pub fn kernel(self, writer: &'a mut dyn Write) -> Result<Self> {
        self.artifact(Artifact::Kernel, writer)
    }

    /// Sets the output writer for the initial RAM filesystem image.
    ///
    /// # Errors
    ///
    /// Returns an error when an initramfs target was already set.
    pub fn initramfs(self, writer: &'a mut dyn Write) -> Result<Self> {
        self.artifact(Artifact::Initramfs, writer)
    }

    /// Sets the output writer for the kernel command-line file.
    ///
    /// # Errors
    ///
    /// Returns an error when a cmdline target was already set.
    pub fn cmdline(self, writer: &'a mut dyn Write) -> Result<Self> {
        self.artifact(Artifact::Cmdline, writer)
    }

    /// Sets the output writer for the unified kernel image.
    ///
    /// # Errors
    ///
    /// Returns an error when a UKI target was already set.
    pub fn uki(self, writer: &'a mut dyn Write) -> Result<Self> {
        self.artifact(Artifact::Uki, writer)
    }

    /// Sets the output writer for the ISO 9660 bootable image.
    ///
    /// # Errors
    ///
    /// Returns an error when an ISO target was already set.
    pub fn iso(self, writer: &'a mut dyn Write) -> Result<Self> {
        self.artifact(Artifact::Iso, writer)
    }

    /// Sets the output writer for the raw disk image.
    ///
    /// # Errors
    ///
    /// Returns an error when a raw target was already set.
    pub fn raw(self, writer: &'a mut dyn Write) -> Result<Self> {
        self.artifact(Artifact::Raw, writer)
    }

    /// Sets the output writer for the overlays tarball.
    ///
    /// # Errors
    ///
    /// Returns an error when an overlays target was already set.
    pub fn overlays(self, writer: &'a mut dyn Write) -> Result<Self> {
        self.artifact(Artifact::Overlays, writer)
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
        self.signing_key = Some(key);
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
    pub async fn build(self, profile: &Profile) -> Result<build::Metadata> {
        let plan = resolve::plan(&self, profile)?;

        build::execute(&plan, profile, self.signing_key, self.targets).await
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arch_conversions() {
        // ARRANGE & ACT & ASSERT
        assert_eq!(Arch::Amd64.as_str(), "amd64");
        assert_eq!(Arch::Arm64.as_str(), "arm64");
        assert_eq!(format!("{}", Arch::Amd64), "amd64");
        assert_eq!(format!("{}", Arch::Arm64), "arm64");
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
            .kernel(&mut buf1)
            .expect("first kernel")
            .kernel(&mut buf2);

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
            .kernel(&mut kernel_buf)
            .expect("kernel")
            .iso(&mut iso_buf)
            .expect("iso");

        // ASSERT
        assert_eq!(request.targets().count(), 2);
        assert_eq!(request.version(), "v1.0.0");
        assert_eq!(request.platform(), Platform::Metal);
        assert_eq!(request.target_arch(), Some(Arch::Amd64));
    }
}
