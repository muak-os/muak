//! User request for image building.

use core::fmt;
use std::io::Write;

use koci::arch::Arch;
use sbolt::keys::SigningPair;

use crate::artifact::Artifact;
use crate::codec::Codec;
use crate::domain::overlay;
use crate::domain::profile::Profile;
use crate::domain::resolution::Resolution;
use crate::error::{Result, WizardError};
use crate::nodes::{disk_layout_annotation, entry_sizes};
use crate::pipeline::context::{BuildContext, TargetWriters};
use crate::pipeline::execute::execute;
use crate::pipeline::plan::plan;
use crate::resolver;

/// A build request expressing what to build and where to write each artifact.
pub struct Request<'a> {
    version: String,
    arch: Option<Arch>,
    codec: Option<Codec>,
    signing: Option<&'a SigningPair<'a>>,
    targets: Vec<(Artifact, &'a mut (dyn Write + Send))>,
}

impl fmt::Debug for Request<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Request")
            .field("version", &self.version)
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
    /// Creates a new request for the given version.
    #[must_use]
    pub fn new<V: Into<String>>(version: V) -> Self {
        Self {
            version: version.into(),
            arch: None,
            codec: None,
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

    /// Sets the transport codec for compressible artifacts.
    ///
    /// When unset, [`Codec::default`] (zstd) applies. An explicitly requested
    /// compressing codec must apply to at least one requested artifact.
    #[must_use]
    pub fn codec(mut self, codec: Codec) -> Self {
        self.codec = Some(codec);

        self
    }

    /// Sets the output writer for an artifact kind.
    ///
    /// # Errors
    ///
    /// Returns an error when a target for the same artifact kind was already set.
    pub fn artifact(mut self, kind: Artifact, writer: &'a mut (dyn Write + Send)) -> Result<Self> {
        if self.targets.iter().any(|item| item.0 == kind) {
            return Err(WizardError::RequestValidation(format!(
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

    /// Rejects explicit compressing codecs that apply to no requested artifact.
    fn validate_codec(&self) -> Result<()> {
        let Some(codec) = self.codec.filter(Codec::compresses) else {
            return Ok(());
        };

        if self
            .targets
            .iter()
            .any(|&(artifact, _)| artifact.supports_codec())
        {
            return Ok(());
        }

        Err(WizardError::RequestValidation(format!(
            "codec {} applies to none of the requested artifacts",
            codec.extension()
        )))
    }

    /// Resolves and builds all requested artifacts.
    ///
    /// # Errors
    ///
    /// Returns an error when resolution, pulling, building, or signing fails.
    pub fn build(self, profile: &Profile) -> Result<crate::Metadata> {
        if self.targets.is_empty() {
            return Err(WizardError::RequestValidation(
                "at least one artifact must be requested".to_owned(),
            ));
        }

        self.validate_codec()?;

        let mut resolution = resolver::plan(&self, profile)?;
        discover_assets(&mut resolution)?;
        discover_layout(&mut resolution)?;
        let profile_bytes = profile.canonical_bytes()?;
        let artifacts: Vec<Artifact> = self.targets.iter().map(|target| target.0).collect();
        let ctx = BuildContext {
            build: resolution.build(),
            profile: &profile_bytes,
            signing: self.signing,
            codec: self.codec.unwrap_or_default(),
        };
        let mut writers = TargetWriters::new(self.targets);
        let graph = plan(&ctx, &artifacts)?;

        execute(graph, &ctx, &mut writers)
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

/// Resolves the disk layout of a resolution onto it.
///
/// # Errors
///
/// Returns an error when the annotations cannot be fetched or name an unknown
/// layout.
pub fn discover_layout(resolution: &mut Resolution) -> Result<()> {
    let layout = match resolution.build().overlay() {
        Some(overlay) => match disk_layout_annotation(&overlay.source, overlay.arch)? {
            Some(name) => disk::layout::Layout::by_name(&name)?,
            None => disk::layout::Layout::Uefi,
        },
        None => disk::layout::Layout::Uefi,
    };

    resolution.set_layout(layout);

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
    }

    #[test]
    fn builder_rejects_duplicate_artifact() {
        // ARRANGE
        let mut buf1 = Vec::new();
        let mut buf2 = Vec::new();

        // ACT
        let result = Request::new("v1.0.0")
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
        let request = Request::new("v1.0.0")
            .arch(Arch::Amd64)
            .artifact(Artifact::Kernel, &mut kernel_buf)
            .expect("kernel")
            .artifact(Artifact::Iso, &mut iso_buf)
            .expect("iso");

        // ASSERT
        assert_eq!(request.targets().count(), 2);
        assert_eq!(request.version(), "v1.0.0");
        assert_eq!(request.target_arch(), Some(Arch::Amd64));
    }

    #[test]
    fn explicit_codec_without_compressible_artifact_is_rejected() {
        // ARRANGE
        let mut iso = Vec::new();
        let request = Request::new("v1.0.0")
            .codec(Codec::Gzip)
            .artifact(Artifact::Iso, &mut iso)
            .expect("iso target");

        // ACT
        let error = request.validate_codec().expect_err("must reject");

        // ASSERT
        assert!(error.to_string().contains("codec gz applies to none"));
    }

    #[test]
    fn explicit_codec_with_raw_artifact_is_accepted() {
        // ARRANGE
        let mut raw = Vec::new();
        let request = Request::new("v1.0.0")
            .codec(Codec::Gzip)
            .artifact(Artifact::Raw, &mut raw)
            .expect("raw target");

        // ACT & ASSERT
        request.validate_codec().expect("raw accepts codecs");
    }

    #[test]
    fn identity_codec_and_default_never_error() {
        // ARRANGE
        let mut iso_none = Vec::new();
        let mut iso_default = Vec::new();
        let none = Request::new("v1.0.0")
            .codec(Codec::None)
            .artifact(Artifact::Iso, &mut iso_none)
            .expect("iso target");
        let unset = Request::new("v1.0.0")
            .artifact(Artifact::Iso, &mut iso_default)
            .expect("iso target");

        // ACT & ASSERT
        none.validate_codec().expect("none is the identity");
        unset.validate_codec().expect("unset uses the default");
    }
}
