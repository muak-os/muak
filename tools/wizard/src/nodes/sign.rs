//! Streams the unsigned UKI for signing through sbolt Authenticode signing.

use std::io::{self, Write};

use sbolt::keys::SigningPair;
use sbolt::signature;

use crate::error::{Result, WizardError};
use crate::nodes::uki;
use crate::pipeline::context::BuildContext;
use crate::pipeline::dependency::Dependency;
use crate::pipeline::execute::NodeReport;
use crate::pipeline::graph::{Graph, NodeId, NodeKind, PortId};
use crate::pipeline::runtime::NodePorts;

pub(crate) const SIGN_INPUT: PortId = PortId(0);
pub(crate) const SIGN_OUTPUT: PortId = PortId(1);

/// The unsigned UKI stream from the Uki node.
pub(crate) fn dependencies() -> Vec<Dependency> {
    vec![Dependency::fixed(
        NodeKind::Uki,
        uki::UKI_OUTPUT,
        SIGN_INPUT,
    )]
}

/// The signed output size of the unsigned input.
pub(crate) fn preflight(
    graph: &mut Graph,
    id: NodeId,
    context: &BuildContext<'_, '_, '_>,
) -> Result<()> {
    let input = graph.node(id)?.input(SIGN_INPUT)?;
    let unsigned = graph.stream(input)?.size;
    let signing = context
        .signing
        .ok_or_else(|| WizardError::BuildError("sign node requires a signing pair".to_owned()))?;
    let total = signed_size(unsigned, signing)?;
    graph.stream_mut(graph.node(id)?.output(SIGN_OUTPUT)?)?.size = total;

    Ok(())
}

/// Streams the unsigned UKI through sbolt into the final output.
pub(crate) fn run(ctx: &BuildContext<'_, '_, '_>, ports: &mut NodePorts) -> Result<NodeReport> {
    let mut input = ports.take(SIGN_INPUT)?.into_input()?;
    let mut output = ports.take(SIGN_OUTPUT)?.into_output()?;
    let signing = ctx
        .signing
        .ok_or_else(|| WizardError::BuildError("sign node requires a signing pair".to_owned()))?;

    let mut counting = CountingWriter {
        inner: &mut output.writer,
        count: 0,
    };
    signature::sign(
        &mut input.reader,
        signing.signer,
        signing.certificate,
        &mut counting,
    )
    .map_err(|e| WizardError::BuildError(format!("sign uki: {e}")))?;
    if counting.count != output.size {
        return Err(WizardError::BuildError(format!(
            "signed uki size mismatch: runtime {} != preflight {}",
            counting.count, output.size,
        )));
    }

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

struct CountingWriter<'a> {
    inner: &'a mut (dyn Write + Send),
    count: u64,
}

impl Write for CountingWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.count = self
            .count
            .saturating_add(u64::try_from(n).unwrap_or(u64::MAX));
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
