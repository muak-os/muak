//! The resolved build and its identity.

use koci::arch::Arch;

use crate::domain::identity::{ProfileId, ReleaseManifestId, ResolutionId};
use crate::request::Platform;

/// The resolved OCI sources and their selected inputs for one build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Sources {
    pub(crate) stub: String,
    pub(crate) installer: String,
    pub(crate) kernel: Kernel,
    pub(crate) overlay: Option<Overlay>,
    pub(crate) extensions: Vec<Extension>,
}

/// The resolved build inputs produced from a request and profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedBuild {
    platform: Platform,
    version: String,
    arch: Arch,
    extensions: Vec<Extension>,
    overlay: Option<Overlay>,
    kernel: Kernel,
    installer: String,
    stub: String,
}

impl ResolvedBuild {
    #[must_use]
    pub(crate) fn new(platform: Platform, version: String, arch: Arch, sources: Sources) -> Self {
        Self {
            platform,
            version,
            arch,
            extensions: sources.extensions,
            overlay: sources.overlay,
            kernel: sources.kernel,
            installer: sources.installer,
            stub: sources.stub,
        }
    }

    /// Returns the resolved platform for the build.
    #[must_use]
    pub const fn platform(&self) -> Platform {
        self.platform
    }

    /// Returns the release manifest version.
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Returns the requested target architecture.
    #[must_use]
    pub const fn arch(&self) -> Arch {
        self.arch
    }

    /// Returns the resolved extension inputs in canonical order.
    #[must_use]
    pub fn extensions(&self) -> &[Extension] {
        &self.extensions
    }

    /// Returns the resolved overlay input when present.
    #[must_use]
    pub fn overlay(&self) -> Option<&Overlay> {
        self.overlay.as_ref()
    }

    /// Returns the resolved kernel source.
    #[must_use]
    pub fn kernel(&self) -> &Kernel {
        &self.kernel
    }

    /// Returns the resolved installer OCI reference.
    #[must_use]
    pub fn installer(&self) -> &str {
        &self.installer
    }

    /// Returns the resolved stub OCI reference.
    #[must_use]
    pub fn stub(&self) -> &str {
        &self.stub
    }
}

/// Complete resolution: the identity split plus the resolved build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    profile_id: ProfileId,
    release_id: ReleaseManifestId,
    id: ResolutionId,
    build: ResolvedBuild,
}

impl Resolution {
    #[must_use]
    pub(crate) fn new(
        profile_id: ProfileId,
        release_id: ReleaseManifestId,
        resolution_id: ResolutionId,
        build: ResolvedBuild,
    ) -> Self {
        Self {
            profile_id,
            release_id,
            id: resolution_id,
            build,
        }
    }

    /// Returns the version-neutral profile identity.
    #[must_use]
    pub const fn profile_id(&self) -> &ProfileId {
        &self.profile_id
    }

    /// Returns the release manifest identity.
    #[must_use]
    pub const fn release_id(&self) -> &ReleaseManifestId {
        &self.release_id
    }

    /// Returns the exact resolution identity.
    #[must_use]
    pub const fn resolution_id(&self) -> &ResolutionId {
        &self.id
    }

    /// Returns the resolved build consumed by the pipeline.
    #[must_use]
    pub const fn build(&self) -> &ResolvedBuild {
        &self.build
    }
}

/// A kernel source resolved from the profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kernel {
    /// Fully-qualified kernel image path.
    image: String,
    /// Versioned OCI reference for the kernel image.
    source: String,
}

impl Kernel {
    #[must_use]
    pub(crate) fn new(image: String, source: String) -> Self {
        Self { image, source }
    }

    /// Returns the fully-qualified kernel image path.
    #[must_use]
    pub fn image(&self) -> &str {
        &self.image
    }

    /// Returns the versioned OCI reference for this kernel image.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }
}

/// A reference to an extension source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extension {
    /// Logical extension name.
    name: String,
    /// Versioned OCI reference for the extension source.
    source: String,
}

impl Extension {
    #[must_use]
    pub(crate) fn new(name: String, source: String) -> Self {
        Self { name, source }
    }

    /// Returns the canonical logical extension name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the versioned OCI reference for this extension.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }
}

/// An overlay source resolved from the profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overlay {
    /// Overlay name inside the OCI image.
    pub name: String,
    /// Logical overlay image name.
    pub image: String,
    /// Versioned OCI reference for the overlay image.
    pub source: String,
    /// Target architecture of the overlay.
    pub arch: Arch,
}

impl Overlay {
    #[must_use]
    pub(crate) fn new(name: String, image: String, source: String, arch: Arch) -> Self {
        Self {
            name,
            image,
            source,
            arch,
        }
    }

    /// Returns the selected overlay name inside the OCI image.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the logical overlay image name.
    #[must_use]
    pub fn image(&self) -> &str {
        &self.image
    }

    /// Returns the versioned OCI reference for this overlay image.
    #[must_use]
    pub fn source_ref(&self) -> &str {
        &self.source
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::identity::ResolutionId;
    use crate::domain::profile::{CustomizationSpec, KernelSpec, Profile};
    use crate::domain::release;

    fn profile() -> Profile {
        let customization = CustomizationSpec::new(vec![]).expect("customization");
        let kernel = KernelSpec::new("muak-os/linux".into()).expect("kernel");
        Profile::new(None, customization, kernel)
    }

    fn manifest() -> release::Manifest {
        release::manifest().expect("manifest")
    }

    #[test]
    fn resolved_build_accessors() {
        // ARRANGE
        let profile = profile();
        let manifest = manifest();
        let build = ResolvedBuild::new(
            Platform::Metal,
            manifest.version().to_owned(),
            Arch::Amd64,
            Sources {
                stub: "ghcr.io/muak-os/stub:latest".into(),
                installer: "ghcr.io/muak-os/installer:latest".into(),
                kernel: Kernel::new(
                    "muak-os/linux".into(),
                    "ghcr.io/muak-os/linux:latest".into(),
                ),
                overlay: None,
                extensions: vec![],
            },
        );
        let resolution = Resolution::new(
            profile.profile_id().expect("profile id"),
            manifest.id().expect("release id"),
            ResolutionId::compute(
                &profile.profile_id().expect("profile id"),
                &manifest.id().expect("release id"),
                "amd64",
                "metal",
                "default",
            ),
            build,
        );

        // ACT / ASSERT
        assert_eq!(resolution.profile_id().to_string().len(), 64);
        assert_eq!(resolution.release_id().to_string().len(), 64);
        assert_eq!(resolution.resolution_id().to_string().len(), 64);
        assert_eq!(resolution.build().platform(), Platform::Metal);
        assert_eq!(
            resolution.build().installer(),
            "ghcr.io/muak-os/installer:latest"
        );
        assert_eq!(resolution.build().stub(), "ghcr.io/muak-os/stub:latest");
    }

    #[test]
    fn resolved_kernel_accessors() {
        // ARRANGE
        let kernel = Kernel::new(
            "ghcr.io/muak-os/linux".into(),
            "ghcr.io/muak-os/linux:v1.0.0".into(),
        );

        // ACT & ASSERT
        assert_eq!(kernel.image(), "ghcr.io/muak-os/linux");
        assert_eq!(kernel.source(), "ghcr.io/muak-os/linux:v1.0.0");
    }

    #[test]
    fn resolved_extension_accessors() {
        // ARRANGE
        let ext = Extension::new("muak-os/qemu".into(), "ghcr.io/muak-os/qemu:v1.0.0".into());

        // ACT & ASSERT
        assert_eq!(ext.name(), "muak-os/qemu");
        assert_eq!(ext.source(), "ghcr.io/muak-os/qemu:v1.0.0");
    }

    #[test]
    fn resolved_overlay_accessors() {
        // ARRANGE
        let ov = Overlay::new(
            "rpi_generic".into(),
            "muak-os/sbc-raspberrypi".into(),
            "ghcr.io/muak-os/sbc/raspberrypi:v1.0.0".into(),
            Arch::Amd64,
        );

        // ACT & ASSERT
        assert_eq!(ov.name(), "rpi_generic");
        assert_eq!(ov.image(), "muak-os/sbc-raspberrypi");
        assert_eq!(ov.source_ref(), "ghcr.io/muak-os/sbc/raspberrypi:v1.0.0");
    }
}
