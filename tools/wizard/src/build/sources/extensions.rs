use crate::error::Result;
use crate::resolve::BuildPlan;
use crate::source::extension;

/// Pulls all requested extension images and buffers their data.
pub(crate) async fn fetch(
    plan: &BuildPlan,
) -> Result<Vec<(String, extension::Metadata, Vec<Vec<u8>>)>> {
    extension::pull(plan.extensions(), &plan.arch()).await
}
