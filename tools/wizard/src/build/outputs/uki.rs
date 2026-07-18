use std::io::{self, Read};

use crate::error::{Result, WizardError};

/// Copies a UKI binary stream to an output writer.
pub(crate) fn uki(reader: &mut dyn Read, writer: &mut dyn std::io::Write) -> Result<()> {
    io::copy(reader, writer)
        .map(|_| ())
        .map_err(|e| WizardError::BuildError(format!("write UKI: {e}")))
}
