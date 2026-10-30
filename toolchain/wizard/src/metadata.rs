//! Metadata produced by one completed build.

use uki::measure::MeasuredSection;

use crate::domain::resolution::BuildIdentity;

/// Artifact build metadata.
#[derive(Debug, Default)]
pub struct Metadata {
    /// PE section measurement records for the built UKI.
    pub sections: Vec<MeasuredSection>,
    /// Build identities, present when a resolution completed.
    pub identity: Option<BuildIdentity>,
}
