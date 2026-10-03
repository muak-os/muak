//! Logical build graph element types.

use crate::artifact::Artifact;
use crate::error::{Result, WizardError};
use crate::nodes::NodeKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct NodeId(pub(crate) usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct StreamId(pub(crate) usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct PortId(pub(crate) usize);

impl PortId {
    /// The port `index` positions after this one, saturating.
    #[must_use]
    pub(crate) const fn offset(self, index: usize) -> Self {
        Self(self.0.saturating_add(index))
    }
}

/// Node-local port identity paired with the logical stream it carries.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PortBinding {
    pub(crate) port: PortId,
    pub(crate) stream: StreamId,
}

/// A logical operation consuming and/or producing streams.
#[derive(Clone, Debug)]
pub(crate) struct Node {
    pub(crate) id: NodeId,
    pub(crate) kind: NodeKind,
    pub(crate) inputs: Vec<PortBinding>,
    pub(crate) outputs: Vec<PortBinding>,
}

impl Node {
    /// Stream bound to a fixed input port.
    pub(crate) fn input(&self, port: PortId) -> Result<StreamId> {
        self.inputs
            .iter()
            .find(|binding| binding.port == port)
            .map(|binding| binding.stream)
            .ok_or_else(|| {
                WizardError::BuildError(format!("node {:?} has no input port {port:?}", self.id))
            })
    }

    /// Stream bound to a fixed output port.
    pub(crate) fn output(&self, port: PortId) -> Result<StreamId> {
        self.outputs
            .iter()
            .find(|binding| binding.port == port)
            .map(|binding| binding.stream)
            .ok_or_else(|| {
                WizardError::BuildError(format!("node {:?} has no output port {port:?}", self.id))
            })
    }

    /// Every port-to-stream input binding, for generic binding and dynamic ports.
    pub(crate) fn input_bindings(&self) -> impl Iterator<Item = &PortBinding> {
        self.inputs.iter()
    }

    /// Every port-to-stream output binding, for generic binding and dynamic ports.
    pub(crate) fn output_bindings(&self) -> impl Iterator<Item = &PortBinding> {
        self.outputs.iter()
    }
}

/// A logical byte flow with a fixed size, one producer, and one destination.
#[derive(Clone, Debug)]
pub(crate) struct Stream {
    pub(crate) id: StreamId,
    pub(crate) name: String,
    pub(crate) size: u64,
    pub(crate) consumers: Vec<NodeId>,
    pub(crate) artifact: Option<Artifact>,
}
