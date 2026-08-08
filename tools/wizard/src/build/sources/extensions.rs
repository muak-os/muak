use crate::error::Result;
use crate::resolve::BuildPlan;
use crate::source::extension;

/// Pulls all requested extensions into opaque image payloads.
pub(crate) async fn fetch(plan: &BuildPlan) -> Result<Vec<mumi::payload::Payload>> {
    extension::pull(plan.extensions(), &plan.arch()).await
}
