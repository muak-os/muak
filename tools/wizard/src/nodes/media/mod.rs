//! ISO and raw disk media image builders.

pub(crate) mod iso;
pub(crate) mod raw;

use esp::FileMeta;
use esp::layout::compute;

use crate::error::{Result, WizardError};
use crate::nodes::{NodeKind, overlay, sign, uki};
use crate::pipeline::context::BuildContext;
use crate::pipeline::dependency::Dependency;
use crate::pipeline::graph::PortId;
use crate::pipeline::runtime::{Endpoint, InputStream, NodePorts};

pub(crate) const MEDIA_UKI: PortId = PortId(0);
pub(crate) const MEDIA_OUTPUT: PortId = PortId(1);
pub(crate) const MEDIA_OVERLAYS_FIRST: PortId = PortId(2);

/// The UKI stream, plus overlay file streams when the profile has an overlay.
pub(crate) fn dependencies(_kind: NodeKind, ctx: &BuildContext<'_, '_, '_>) -> Vec<Dependency> {
    let uki = if ctx.signing.is_some() {
        Dependency::fixed(NodeKind::Sign, sign::SIGN_OUTPUT, MEDIA_UKI)
    } else {
        Dependency::fixed(NodeKind::Uki, uki::UKI_OUTPUT, MEDIA_UKI)
    };
    let mut dependencies = vec![uki];
    if ctx.build.overlay().is_some() {
        dependencies.push(Dependency::many(
            NodeKind::OverlayPull,
            overlay::pull::PULL_OUTPUTS_FIRST,
            MEDIA_OVERLAYS_FIRST,
        ));
    }

    dependencies
}

pub(crate) fn media_inputs<'a>(
    ports: &mut NodePorts<'a>,
) -> Result<(InputStream<'a>, Vec<InputStream<'a>>)> {
    let uki = ports.take(MEDIA_UKI)?.into_input()?;
    let overlays = Endpoint::into_inputs(
        ports
            .take_from(MEDIA_OVERLAYS_FIRST, None)?
            .into_iter()
            .map(|(_, endpoint)| endpoint),
    )?;

    Ok((uki, overlays))
}

pub(crate) fn media_layout<'a>(
    ctx: &BuildContext<'_, '_, '_>,
    uki: &InputStream<'a>,
    overlays: &[InputStream<'a>],
) -> Result<esp::layout::Layout<'a>> {
    let mut file_metas = Vec::with_capacity(overlays.len().saturating_add(1));
    file_metas.push(FileMeta::new(
        crate::arch::esp(ctx.build.arch()).boot_path(),
        uki.size,
    ));
    file_metas.extend(
        overlays
            .iter()
            .map(|input| FileMeta::new(input.name, input.size)),
    );

    compute(&file_metas).map_err(|e| WizardError::BuildError(format!("compute ESP layout: {e}")))
}
