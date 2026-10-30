//! The resolved build and its identity.

use disk::layout::Layout;
use koci::arch::Arch;

use super::catalog::Catalog;
use super::input::{Extension, Kernel, Overlay};
use super::overlay::Asset;
use crate::domain::identity::{ProfileId, ResolutionId, ResolvedInput};

/// The resolved build inputs produced from a request and profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedBuild {
    version: String,
    arch: Arch,
    extensions: Vec<Extension>,
    overlay: Option<Overlay>,
    overlay_assets: Option<Vec<Asset>>,
    layout: Option<Layout>,
    kernel: Kernel,
    installer: String,
    stub: String,
}

impl ResolvedBuild {
    #[must_use]
    pub(crate) fn new(version: String, arch: Arch, kernel: Kernel) -> Self {
        Self {
            version,
            arch,
            extensions: Vec::new(),
            overlay: None,
            overlay_assets: None,
            layout: None,
            kernel,
            installer: String::new(),
            stub: String::new(),
        }
    }

    /// Attaches the resolved OCI sources to the build.
    #[must_use]
    pub(crate) fn with_sources(
        mut self,
        stub: String,
        installer: String,
        overlay: Option<Overlay>,
        extensions: Vec<Extension>,
    ) -> Self {
        self.stub = stub;
        self.installer = installer;
        self.overlay = overlay;
        self.extensions = extensions;

        self
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

    /// Returns the number of initramfs payload layers this build contributes:
    /// the kernel modules layer plus one payload per extension.
    #[must_use]
    pub fn payload_layer_count(&self) -> usize {
        self.extensions.len().saturating_add(1)
    }

    /// Returns the resolved overlay input when present.
    #[must_use]
    pub fn overlay(&self) -> Option<&Overlay> {
        self.overlay.as_ref()
    }

    /// Returns the discovered overlay assets when the build has an overlay.
    #[must_use]
    pub fn overlay_assets(&self) -> Option<&[Asset]> {
        self.overlay_assets.as_deref()
    }

    /// Attaches the discovered overlay assets to the build.
    pub(crate) fn set_overlay_assets(&mut self, assets: Option<Vec<Asset>>) {
        self.overlay_assets = assets;
    }

    /// Returns the resolved disk layout, defaulting to UEFI.
    #[must_use]
    pub fn layout(&self) -> Layout {
        self.layout.unwrap_or(Layout::Uefi)
    }

    /// Attaches the resolved disk layout to the build.
    pub(crate) fn set_layout(&mut self, layout: Layout) {
        self.layout = Some(layout);
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
    id: ResolutionId,
    inputs: Vec<ResolvedInput>,
    catalog: Catalog,
    build: ResolvedBuild,
}

impl Resolution {
    #[must_use]
    pub(crate) fn new(
        profile_id: ProfileId,
        resolution_id: ResolutionId,
        inputs: Vec<ResolvedInput>,
        catalog: Catalog,
        build: ResolvedBuild,
    ) -> Self {
        Self {
            profile_id,
            id: resolution_id,
            inputs,
            catalog,
            build,
        }
    }

    /// Returns the version-neutral profile identity.
    #[must_use]
    pub const fn profile_id(&self) -> &ProfileId {
        &self.profile_id
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

    /// Returns the frozen input records that produced the resolution identity.
    #[must_use]
    pub fn inputs(&self) -> &[ResolvedInput] {
        &self.inputs
    }

    /// Returns the core catalog image this resolution was derived from.
    #[must_use]
    pub const fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    /// Attaches the discovered overlay assets to the resolved build.
    pub(crate) fn set_overlay_assets(&mut self, assets: Option<Vec<Asset>>) {
        self.build.set_overlay_assets(assets);
    }

    /// Attaches the resolved disk layout to the resolved build.
    pub(crate) fn set_layout(&mut self, layout: Layout) {
        self.build.set_layout(layout);
    }
}

/// The exact identities and inputs of a completed build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildIdentity {
    profile: ProfileId,
    resolution: ResolutionId,
    release: String,
    catalog: Catalog,
    inputs: Vec<ResolvedInput>,
}

impl BuildIdentity {
    #[must_use]
    pub(crate) fn new(
        profile: ProfileId,
        resolution: ResolutionId,
        release: String,
        catalog: Catalog,
        inputs: Vec<ResolvedInput>,
    ) -> Self {
        Self {
            profile,
            resolution,
            release,
            catalog,
            inputs,
        }
    }

    /// Returns the version-neutral profile identity.
    #[must_use]
    pub const fn profile(&self) -> &ProfileId {
        &self.profile
    }

    /// Returns the identity of the exact resolved build.
    #[must_use]
    pub const fn resolution(&self) -> &ResolutionId {
        &self.resolution
    }

    /// Returns the concrete release line the build resolved.
    #[must_use]
    pub fn release(&self) -> &str {
        &self.release
    }

    /// Returns the core catalog image the resolution was derived from.
    #[must_use]
    pub const fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    /// Returns the frozen input records that produced the resolution identity.
    #[must_use]
    pub fn inputs(&self) -> &[ResolvedInput] {
        &self.inputs
    }
}

impl From<&Resolution> for BuildIdentity {
    fn from(resolution: &Resolution) -> Self {
        Self::new(
            resolution.profile_id().clone(),
            resolution.resolution_id().clone(),
            resolution.build().version().to_owned(),
            resolution.catalog().clone(),
            resolution.inputs().to_vec(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::identity::ResolutionId;
    use crate::domain::profile::{CustomizationSpec, KernelSpec, Profile};

    fn profile() -> Profile {
        let customization = CustomizationSpec::new(vec![]).expect("customization");
        let kernel = KernelSpec::new("muak-os/linux".into()).expect("kernel");
        Profile::new(None, customization, kernel)
    }

    #[test]
    fn resolved_build_accessors() {
        // ARRANGE
        let profile = profile();
        let build = ResolvedBuild::new(
            "v1.0.0".to_owned(),
            Arch::Amd64,
            Kernel::new(
                "muak-os/linux".into(),
                "ghcr.io/muak-os/linux:v1.0.0".into(),
            ),
        )
        .with_sources(
            "ghcr.io/muak-os/stub:v1.0.0".into(),
            "ghcr.io/muak-os/installer:v1.0.0".into(),
            None,
            vec![],
        );
        let catalog = Catalog::new("ghcr.io/muak-os/core".into(), "sha256:abcd".into());
        let resolution = Resolution::new(
            profile.profile_id().expect("profile id"),
            ResolutionId::compute(
                &profile.profile_id().expect("profile id"),
                &[],
                "amd64",
                "default",
            ),
            Vec::new(),
            catalog,
            build,
        );

        // ACT / ASSERT
        assert_eq!(resolution.profile_id().to_string().len(), 64);
        assert_eq!(resolution.resolution_id().to_string().len(), 64);
        assert_eq!(resolution.inputs(), []);
        assert_eq!(
            resolution.catalog().pinned(),
            "ghcr.io/muak-os/core@sha256:abcd"
        );
        assert_eq!(
            resolution.build().installer(),
            "ghcr.io/muak-os/installer:v1.0.0"
        );
        assert_eq!(resolution.build().stub(), "ghcr.io/muak-os/stub:v1.0.0");
    }
}
