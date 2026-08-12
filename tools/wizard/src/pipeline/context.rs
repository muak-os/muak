//! Immutable build inputs shared by preflight and the node runners.

use sbolt::keys::SigningPair;

use crate::resolve::BuildPlan;

/// Immutable build inputs, passed explicitly to preflight and runners.
pub(crate) struct BuildContext<'data, 'sign> {
    pub(crate) plan: &'data BuildPlan,
    pub(crate) profile: &'data [u8],
    pub(crate) signing: Option<&'sign SigningPair<'sign>>,
}
