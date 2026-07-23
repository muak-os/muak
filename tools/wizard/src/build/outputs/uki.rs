use std::io::{self, Read};

use crate::error::{Result, WizardError};

pub(crate) fn uki(reader: &mut dyn Read, writer: &mut (dyn std::io::Write + Send)) -> Result<()> {
    io::copy(reader, writer)
        .map(|_| ())
        .map_err(|e| WizardError::BuildError(format!("write UKI: {e}")))
}
