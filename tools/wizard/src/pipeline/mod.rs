//! Build pipeline: planning, normalization, preflight sizing, preparation,
//! and execution of the logical build graph.

pub(crate) mod context;
pub(crate) mod dependency;
pub(crate) mod execute;
pub(crate) mod graph;
pub(crate) mod node;
pub(crate) mod normalize;
pub(crate) mod plan;
pub(crate) mod preflight;
pub(crate) mod prepare;
pub(crate) mod runtime;
