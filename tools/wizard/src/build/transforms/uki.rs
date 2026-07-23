use std::io;
use std::os::unix::net::UnixStream;

use sbolt::keys::SigningPair;
use yuki::pe::section::Section;

use crate::build::uki;
use crate::error::{Result, WizardError};
use crate::source::installer::Metadata;

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

pub(crate) fn open(
    meta: &Metadata,
    tail_size: u64,
    tail_pipe: Option<UnixStream>,
    signing: Option<&SigningPair<'_>>,
) -> Result<Uki> {
    let mut build = uki::build(meta, tail_size, signing)?;
    if let Some(mut tail) = tail_pipe {
        io::copy(&mut tail, &mut build.tail_w)
            .map_err(|e| WizardError::BuildError(format!("write tail to UKI pipe: {e}")))?;
    }

    Ok(Uki { build })
}

pub(crate) struct UkiOutcome {
    pub reader: UnixStream,
    pub size: u64,
    pub sections: Vec<Section>,
}

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
