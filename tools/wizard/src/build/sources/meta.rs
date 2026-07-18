use crate::error::Result;
use crate::resolve::BuildPlan;
use crate::source::installer;

/// Fetches file size metadata from the installer OCI image.
pub(crate) async fn fetch(plan: &BuildPlan) -> Result<installer::Metadata> {
    installer::metadata(plan.installer(), &plan.arch(), None).await
}
