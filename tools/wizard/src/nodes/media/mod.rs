//! ISO and raw disk media image builders.

pub(crate) mod iso;
pub(crate) mod raw;

use esp::FileMeta;
use esp::layout::compute;

use crate::domain::overlay::Asset;
use crate::error::{Result, WizardError};
use crate::nodes::{NodeKind, overlay, sign, uki};
use crate::pipeline::context::BuildContext;
use crate::pipeline::dependency::Dependency;
use crate::pipeline::node::PortId;
use crate::pipeline::runtime::{Endpoint, InputStream, NodePorts};

pub(crate) const MEDIA_UKI: PortId = PortId(0);
pub(crate) const MEDIA_OUTPUT: PortId = PortId(1);
pub(crate) const MEDIA_OVERLAYS_FIRST: PortId = PortId(2);

/// The UKI stream, plus one stream per overlay asset when the build has overlay assets.
pub(crate) fn dependencies(_kind: NodeKind, ctx: &BuildContext<'_, '_>) -> Vec<Dependency> {
    let uki = if ctx.signing.is_some() {
        Dependency::new(NodeKind::Sign, sign::SIGN_OUTPUT, MEDIA_UKI)
    } else {
        Dependency::new(NodeKind::Uki, uki::UKI_OUTPUT, MEDIA_UKI)
    };
    let mut dependencies = vec![uki];
    if let Some(assets) = ctx.build.overlay_assets() {
        for index in 0..assets.len() {
            dependencies.push(Dependency::new(
                NodeKind::OverlayPull,
                overlay::pull::PULL_OUTPUTS_FIRST.offset(index),
                MEDIA_OVERLAYS_FIRST.offset(index),
            ));
        }
    }

    dependencies
}

pub(crate) fn media_inputs<'name>(
    ports: &mut NodePorts<'name, '_>,
) -> Result<(InputStream<'name>, Vec<InputStream<'name>>)> {
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
    ctx: &BuildContext<'_, '_>,
    uki: &InputStream<'a>,
    assets: &'a [Asset],
) -> Result<esp::layout::Layout<'a>> {
    let mut file_metas = Vec::with_capacity(assets.len().saturating_add(1));
    file_metas.push(FileMeta::new(
        crate::arch::esp(ctx.build.arch()).boot_path(),
        uki.size,
    ));
    for asset in assets {
        if let Asset::EspFile { ref path, size } = *asset {
            file_metas.push(FileMeta::new(path, size));
        }
    }

    compute(&file_metas).map_err(|e| WizardError::BuildError(format!("compute ESP layout: {e}")))
}
