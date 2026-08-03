use std::collections::HashMap;
use std::os::unix::net::UnixStream;

use crate::build::archive;
use crate::error::{Result, WizardError};
use crate::source::extension::{BufferedReader, Metadata as ExtensionMetadata};

pub(crate) struct Tail {
    pub size: u64,
    pub reader: UnixStream,
}

pub(crate) fn build(
    ext_data: Vec<(String, ExtensionMetadata, BufferedReader)>,
    profile_bytes: &[u8],
) -> Result<Tail> {
    let mut images_by_path: HashMap<String, mumi::image::Image> = HashMap::new();
    let mut entries: Vec<ramune::Entry> = Vec::new();

    for (name, meta, mut reader) in ext_data {
        let image = archive::build_extension_image(&name, &meta, &mut reader)?;
        let path = format!(
            "extensions/{}.erofs",
            archive::extension_archive_name(&name)
        );
        let len = image.len();
        images_by_path.insert(path.clone(), image);
        entries.push(ramune::Entry {
            path,
            mode: 0o100_644,
            len,
        });
    }

    if !profile_bytes.is_empty() {
        entries.push(ramune::Entry {
            path: "profile.toml".to_owned(),
            mode: 0o100_644,
            len: u64::try_from(profile_bytes.len()).unwrap_or(u64::MAX),
        });
    }

    let size = archive::tail_exact_size(&entries);
    let (mut writer, reader) = UnixStream::pair()
        .map_err(|e| WizardError::BuildError(format!("create tail pipe: {e}")))?;
    archive::build_tail(&images_by_path, &mut entries, profile_bytes, &mut writer)?;

    Ok(Tail { size, reader })
}
