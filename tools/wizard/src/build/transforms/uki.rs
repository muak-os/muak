use std::os::unix::net::UnixStream;

use sbolt::keys::SigningPair;
use yuki::pe::section::Section;

use crate::build::archive::TailParts;
use crate::build::uki;
use crate::error::{Result, WizardError};
use crate::source::installer::Metadata;

/// An open UKI build session.
pub(crate) struct Uki {
    build: uki::Build,
}

impl Uki {
    pub(crate) fn stub_w(&mut self) -> Option<UnixStream> {
        self.build.stub_w.try_clone().ok()
    }

    pub(crate) fn data_w(&mut self) -> Option<UnixStream> {
        self.build.data_w.try_clone().ok()
    }
}

/// Opens UKI assembly pipes and spawns the yuki build task.
pub(crate) fn open(
    meta: &Metadata,
    tail_size: u64,
    tail_parts: Option<&TailParts>,
    signing_key: Option<&SigningPair<'_>>,
) -> Result<Uki> {
    let mut build = uki::build(meta, tail_size, signing_key)?;
    if let Some(tail) = tail_parts {
        uki::write_tail(&mut build, tail)?;
    }
    Ok(Uki { build })
}

/// The result of collecting a completed UKI build.
pub(crate) struct UkiOutcome {
    pub reader: UnixStream,
    pub size: u64,
    pub sections: Vec<Section>,
}

/// Awaits the yuki build task and returns the collected UKI binary.
pub(crate) async fn collect(uki: Uki) -> Result<UkiOutcome> {
    let (reader, size, handle) = uki::collect(uki.build);
    let sections = handle
        .await
        .map_err(|e| WizardError::BuildError(format!("join UKI build task: {e}")))?
        .map_err(|e| WizardError::BuildError(format!("UKI build failed: {e}")))?;
    Ok(UkiOutcome {
        reader,
        size,
        sections,
    })
}
