//! Streams the unsigned UKI for signing through sbolt Authenticode signing.

use sbolt::keys::SigningPair;
use sbolt::signature;

use crate::error::{Result, WizardError};
use crate::nodes::uki;
use crate::nodes::{NodeDescriptor, NodeKind};
use crate::pipeline::context::BuildContext;
use crate::pipeline::dependency::Dependency;
use crate::pipeline::execute::NodeReport;
use crate::pipeline::graph::{Graph, NodeId, PortId};
use crate::pipeline::runtime::NodePorts;

pub(crate) const SIGN_INPUT: PortId = PortId(0);
pub(crate) const SIGN_OUTPUT: PortId = PortId(1);

pub(crate) const DESCRIPTOR: NodeDescriptor = NodeDescriptor {
    dependencies,
    preflight,
    run,
};

/// The unsigned UKI stream from the Uki node.
fn dependencies(_kind: NodeKind, _ctx: &BuildContext<'_, '_, '_>) -> Vec<Dependency> {
    vec![Dependency::new(NodeKind::Uki, uki::UKI_OUTPUT, SIGN_INPUT)]
}

/// The signed output size of the unsigned input.
fn preflight(graph: &mut Graph, id: NodeId, ctx: &BuildContext<'_, '_, '_>) -> Result<()> {
    let input = graph.node(id)?.input(SIGN_INPUT)?;
    let unsigned = graph.stream(input)?.size;
    let signing = ctx
        .signing
        .ok_or_else(|| WizardError::BuildError("sign node requires a signing pair".to_owned()))?;
    let total = signed_size(unsigned, signing)?;
    let output = graph.stream_mut(graph.node(id)?.output(SIGN_OUTPUT)?)?;
    output.size = total;
    "uki.efi".clone_into(&mut output.name);

    Ok(())
}

/// Streams the unsigned UKI through sbolt into the final output.
fn run(
    _kind: NodeKind,
    ports: &mut NodePorts<'_>,
    ctx: &BuildContext<'_, '_, '_>,
) -> Result<NodeReport> {
    let mut input = ports.take(SIGN_INPUT)?.into_input()?;
    let mut output = ports.take(SIGN_OUTPUT)?.into_output()?;
    let signing = ctx
        .signing
        .ok_or_else(|| WizardError::BuildError("sign node requires a signing pair".to_owned()))?;

    signature::sign(
        &mut input.reader,
        signing.signer,
        signing.certificate,
        &mut output.writer,
    )
    .map_err(|e| WizardError::BuildError(format!("sign uki: {e}")))?;

    Ok(NodeReport::Empty)
}

fn signed_size(unsigned: u64, signing: &SigningPair<'_>) -> Result<u64> {
    let aligned = unsigned
        .checked_add(7)
        .ok_or_else(|| WizardError::BuildError("uki alignment overflow".to_owned()))?
        & !7;
    let cert_size = u64::try_from(
        signature::cert_table_size(signing.certificate)
            .map_err(|e| WizardError::BuildError(format!("certificate table size: {e}")))?,
    )
    .map_err(|e| WizardError::BuildError(format!("certificate size overflow: {e}")))?;

    aligned
        .checked_add(cert_size)
        .ok_or_else(|| WizardError::BuildError("signed uki size overflow".to_owned()))
}
