use std::os::unix::net::UnixStream;

use crate::build::archive;
use crate::error::{Result, WizardError};
use crate::source::extension::Metadata as ExtensionMetadata;

pub(crate) struct Tail {
    pub size: u64,
    pub reader: UnixStream,
}

pub(crate) fn build(
    ext_data: &[(String, ExtensionMetadata, Vec<Vec<u8>>)],
    profile_bytes: &[u8],
) -> Result<Tail> {
    let parts = archive::prepare_tail_parts(ext_data, profile_bytes)?;
    let size = archive::tail_exact_size(&parts);
    let (mut writer, reader) = UnixStream::pair()
        .map_err(|e| WizardError::BuildError(format!("create tail pipe: {e}")))?;
    archive::build_tail_from_parts(&parts, &mut writer)?;
    Ok(Tail { size, reader })
}
