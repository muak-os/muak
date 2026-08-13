//! Streams pre-planned opaque extension payloads.

use crate::error::{Result, WizardError};
use crate::pipeline::context::BuildContext;
use crate::pipeline::dependency::Dependency;
use crate::pipeline::execute::NodeReport;
use crate::pipeline::graph::{Graph, NodeId, PortId};
use crate::pipeline::runtime::{DynWriter, Endpoint, NodePorts};
use crate::resolve::BuildPlan;
use crate::source::extension::pull;

pub(crate) const FIRST_OUTPUT: PortId = PortId(0);

/// Source node meaning no dependencies.
pub(crate) fn dependencies() -> Vec<Dependency> {
    Vec::new()
}

/// Pulls and plans the extension payloads exactly once, returning the
/// `Planned` list in canonical source order for the other nodes.
pub(crate) async fn preflight(
    graph: &mut Graph,
    id: NodeId,
    context: &BuildContext<'_, '_>,
) -> Result<Vec<mumi::payload::Planned>> {
    let plan = context.plan;

    let mut payloads = pull(plan.extensions(), &plan.arch()).await?;
    let planned = mumi::payload::plan(&mut payloads, &config())
        .map_err(|e| WizardError::BuildError(format!("plan extension payloads: {e}")))?;

    let bindings = graph
        .node(id)?
        .output_bindings()
        .copied()
        .collect::<Vec<_>>();
    if bindings.len() != planned.len() {
        return Err(WizardError::BuildError(format!(
            "extension output/payload count mismatch: {} != {}",
            bindings.len(),
            planned.len(),
        )));
    }
    for (binding, payload) in bindings.iter().zip(&planned) {
        graph.stream_mut(binding.stream)?.size = payload.meta().size;
    }

    Ok(planned)
}

/// Streams each planned payload into its output stream without re-pulling
/// or re-planning. The payload format is opaque to wizard.
pub(crate) fn run(
    payloads: &[mumi::payload::Planned],
    ports: &mut NodePorts<'_>,
) -> Result<NodeReport> {
    let mut outputs = Endpoint::into_outputs(
        ports
            .take_from(FIRST_OUTPUT, Some(payloads.len()))?
            .into_iter()
            .map(|(_, endpoint)| endpoint),
    )?;
    for (payload, output) in payloads.iter().zip(outputs.iter_mut()) {
        payload
            .write(&mut DynWriter::new(output.writer()))
            .map_err(|e| WizardError::BuildError(format!("stream extension payload: {e}")))?;
    }

    Ok(NodeReport::Empty)
}

/// One payload stream per extension, in canonical source order.
pub(crate) fn output_count(build: &BuildPlan) -> usize {
    build.extensions().len()
}

fn config() -> mumi::image::BuildConfig {
    mumi::image::BuildConfig {
        compression_level: mumi::DEFAULT_ZSTD_COMPRESSION_LEVEL,
        file_contexts: None,
    }
}
