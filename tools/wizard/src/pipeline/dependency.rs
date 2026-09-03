//! Node dependency declarations.

use crate::nodes::NodeKind;
use crate::pipeline::node::PortId;

/// One declared input of a node: the stream produced by `producer` on
/// `producer_port`, bound to the consumer's `consumer_port`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Dependency {
    pub(crate) producer: NodeKind,
    pub(crate) producer_port: PortId,
    pub(crate) consumer_port: PortId,
}

impl Dependency {
    /// One stream between the two ports.
    #[must_use]
    pub(crate) const fn new(
        producer: NodeKind,
        producer_port: PortId,
        consumer_port: PortId,
    ) -> Self {
        Self {
            producer,
            producer_port,
            consumer_port,
        }
    }
}
