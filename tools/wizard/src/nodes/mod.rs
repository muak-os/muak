//! Build pipeline node runners.

use alloc::collections::BTreeMap;

pub(crate) mod initramfs;
pub(crate) mod installer;
pub(crate) mod kernel;
pub(crate) mod layers;
pub(crate) mod media;
pub(crate) mod overlay;
pub(crate) mod sign;
pub(crate) mod stub;
pub(crate) mod uki;

use koci::arch::Arch;
use koci::pull;

use crate::artifact::Artifact;
use crate::error::{Result, WizardError};
use crate::pipeline::context::BuildContext;
use crate::pipeline::dependency::Dependency;
use crate::pipeline::execute::NodeReport;
use crate::pipeline::graph::Graph;
use crate::pipeline::node::{NodeId, PortId};
use crate::pipeline::runtime::NodePorts;

/// Manifest annotation carrying the per-file sizes of a producer image.
const SIZES_ANNOTATION: &str = "dev.muak.sizes";

/// Generates the `NodeKind` enum, its `ALL` table, and the descriptor dispatch from the single node registry.
macro_rules! node_registry {
    ( $( $kind:ident => $descriptor:path ),+ $(,)? ) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub(crate) enum NodeKind {
            $( $kind, )+
        }

        impl NodeKind {
            pub(crate) const ALL: &'static [NodeKind] = &[ $( NodeKind::$kind, )+ ];
        }

        pub(crate) fn descriptor(kind: NodeKind) -> &'static NodeDescriptor {
            match kind {
                $( NodeKind::$kind => &$descriptor, )+
            }
        }
    };
}

node_registry! {
    InstallerPull => installer::DESCRIPTOR,
    StubPull => stub::DESCRIPTOR,
    KernelPull => kernel::DESCRIPTOR,
    LayerPayloads => layers::DESCRIPTOR,
    InitramfsTail => initramfs::tail::DESCRIPTOR,
    Concat => initramfs::concat::DESCRIPTOR,
    Uki => uki::DESCRIPTOR,
    Sign => sign::DESCRIPTOR,
    Iso => media::iso::DESCRIPTOR,
    Raw => media::raw::DESCRIPTOR,
    OverlayPull => overlay::pull::DESCRIPTOR,
    OverlayTar => overlay::tar::DESCRIPTOR,
}

/// One node kind's full contract.
pub(crate) struct NodeDescriptor {
    pub(crate) dependencies: fn(NodeKind, &BuildContext<'_, '_>) -> Vec<Dependency>,
    pub(crate) produces: fn(NodeKind, &BuildContext<'_, '_>) -> Vec<(PortId, Artifact)>,
    pub(crate) preflight: fn(&mut Graph, NodeId, &BuildContext<'_, '_>) -> Result<()>,
    pub(crate) run:
        fn(NodeKind, &mut NodePorts<'_, '_>, &BuildContext<'_, '_>) -> Result<NodeReport>,
}

/// Declared inputs of a planned kind, read from its descriptor.
pub(crate) fn dependencies(kind: NodeKind, ctx: &BuildContext<'_, '_>) -> Vec<Dependency> {
    (descriptor(kind).dependencies)(kind, ctx)
}

/// Declared artifact outputs of a planned kind: which port yields which artifact.
pub(crate) fn produces(kind: NodeKind, ctx: &BuildContext<'_, '_>) -> Vec<(PortId, Artifact)> {
    (descriptor(kind).produces)(kind, ctx)
}

/// Byte size of every file entry of an image, from its sizes manifest annotation.
///
/// # Errors
///
/// Returns an error when the annotations cannot be fetched or the sizes
/// annotation is missing or malformed.
pub(crate) fn entry_sizes(reference: &str, arch: Arch) -> Result<BTreeMap<String, u64>> {
    let annotations = pull::annotations(reference, &arch, None)
        .map_err(|e| WizardError::BuildError(format!("fetch {reference} annotations: {e}")))?;

    parse_sizes(reference, &annotations)
}

fn parse_sizes(
    reference: &str,
    annotations: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, u64>> {
    let Some(raw) = annotations.get(SIZES_ANNOTATION) else {
        let available = annotations.keys().cloned().collect::<Vec<_>>().join(", ");
        let available = if available.is_empty() {
            "none".to_owned()
        } else {
            available
        };

        return Err(WizardError::BuildError(format!(
            "missing {SIZES_ANNOTATION} annotation on {reference} (present: {available})"
        )));
    };

    serde_json::from_str(raw).map_err(|e| {
        WizardError::BuildError(format!(
            "malformed {SIZES_ANNOTATION} annotation on {reference}: expected a JSON object \
             mapping entry paths to byte sizes: {e}"
        ))
    })
}

#[cfg(test)]
mod tests {
    use alloc::collections::BTreeMap;

    use koci::arch::Arch;
    use sbolt::keys::SigningPair;
    use sbolt::keys::cert::generate_pk;

    use super::*;
    use crate::domain::resolution::Kernel;
    use crate::domain::resolution::ResolvedBuild;
    use crate::request::Platform;

    fn sizes_annotation(value: &str) -> BTreeMap<String, String> {
        BTreeMap::from([(SIZES_ANNOTATION.to_owned(), value.to_owned())])
    }

    #[test]
    fn parse_sizes_reads_entry_sizes() {
        // ARRANGE
        let annotations = sizes_annotation(r#"{"vmlinuz":12,"cmdline":3}"#);

        // ACT
        let sizes = parse_sizes("reg.io/img:tag", &annotations).expect("parse sizes");

        // ASSERT
        assert_eq!(sizes.get("vmlinuz"), Some(&12));
        assert_eq!(sizes.get("cmdline"), Some(&3));
    }

    #[test]
    fn parse_sizes_rejects_missing_annotation_and_lists_present_keys() {
        // ARRANGE
        let annotations = BTreeMap::from([("dev.muak.sig".to_owned(), "AA".to_owned())]);

        // ACT
        let error = parse_sizes("reg.io/img:tag", &annotations).expect_err("missing must fail");

        // ASSERT
        assert!(error.to_string().contains(SIZES_ANNOTATION));
        assert!(error.to_string().contains("dev.muak.sig"));
    }

    #[test]
    fn parse_sizes_reports_missing_annotation_without_present_keys() {
        // ARRANGE / ACT
        let error = parse_sizes("reg.io/img:tag", &BTreeMap::new()).expect_err("missing must fail");

        // ASSERT
        assert!(error.to_string().contains("(present: none)"));
    }

    #[test]
    fn parse_sizes_rejects_malformed_json() {
        // ARRANGE
        let annotations = sizes_annotation("not json");

        // ACT / ASSERT
        parse_sizes("reg.io/img:tag", &annotations).expect_err("malformed must fail");
    }

    #[test]
    fn parse_sizes_rejects_non_numeric_size() {
        // ARRANGE
        let annotations = sizes_annotation(r#"{"vmlinuz":"big"}"#);

        // ACT / ASSERT
        parse_sizes("reg.io/img:tag", &annotations).expect_err("non-numeric must fail");
    }

    fn build_plan() -> ResolvedBuild {
        ResolvedBuild::new(
            Platform::Metal,
            "v1.0.0".to_owned(),
            Arch::Amd64,
            Kernel::new(
                "ghcr.io/muak-os/linux".to_owned(),
                "ghcr.io/muak-os/linux:v1.0.0".to_owned(),
            ),
        )
        .with_sources(
            "ghcr.io/muak-os/stub:v1.0.0".to_owned(),
            "ghcr.io/muak-os/installer:v1.0.0".to_owned(),
            None,
            Vec::new(),
        )
    }

    fn context(build: &ResolvedBuild) -> BuildContext<'_, '_> {
        BuildContext {
            build,
            profile: b"",
            signing: None,
        }
    }

    #[test]
    fn produces_declares_artifact_outputs_through_the_descriptor_table() {
        // ARRANGE
        let build = build_plan();
        let ctx = context(&build);
        let routes = [
            (NodeKind::KernelPull, (kernel::KERNEL, Artifact::Kernel)),
            (NodeKind::KernelPull, (kernel::CMDLINE, Artifact::Cmdline)),
            (
                NodeKind::Concat,
                (initramfs::concat::CONCAT_OUTPUT, Artifact::Initramfs),
            ),
            (NodeKind::Uki, (uki::UKI_OUTPUT, Artifact::Uki)),
            (NodeKind::Iso, (media::MEDIA_OUTPUT, Artifact::Iso)),
            (NodeKind::Raw, (media::MEDIA_OUTPUT, Artifact::Raw)),
            (
                NodeKind::OverlayTar,
                (overlay::tar::TAR_OUTPUT, Artifact::Overlays),
            ),
        ];

        for (kind, (port, artifact)) in routes {
            // ACT
            let declared = produces(kind, &ctx);

            // ASSERT
            assert!(
                declared.contains(&(port, artifact)),
                "{kind:?} must declare {artifact} on port {port:?}"
            );
        }
    }

    #[test]
    fn unsigned_planning_produces_the_uki_from_the_uki_node_only() {
        // ARRANGE
        let build = build_plan();
        let ctx = context(&build);

        // ACT
        let sources: Vec<_> = NodeKind::ALL
            .iter()
            .flat_map(|kind| produces(*kind, &ctx))
            .filter(|&(_, artifact)| artifact == Artifact::Uki)
            .collect();

        // ASSERT
        assert_eq!(
            sources,
            vec![(uki::UKI_OUTPUT, Artifact::Uki)],
            "unsigned planning must route Uki through the Uki node only"
        );
    }

    #[test]
    fn signed_planning_produces_the_uki_from_the_sign_node_only() {
        // ARRANGE
        let build = build_plan();
        let (signer, certificate) = generate_pk("muak-test").expect("generate signing pair");
        let signing = SigningPair {
            signer: &signer,
            certificate: &certificate,
        };
        let ctx = BuildContext {
            build: &build,
            profile: b"",
            signing: Some(&signing),
        };

        // ACT
        let sources: Vec<_> = NodeKind::ALL
            .iter()
            .flat_map(|kind| produces(*kind, &ctx))
            .filter(|&(_, artifact)| artifact == Artifact::Uki)
            .collect();

        // ASSERT
        assert_eq!(
            sources,
            vec![(sign::SIGN_OUTPUT, Artifact::Uki)],
            "signed planning must route Uki through the Sign node only"
        );
    }
}
