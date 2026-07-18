use std::collections::HashMap;
use std::os::unix::net::UnixStream;

use esp::FileMeta;
use tokio::task::JoinHandle;

use crate::error::{Result, WizardError};
use crate::resolve::BuildPlan;
use crate::source::overlay;

/// Pipes and metadata for streaming overlay files.
pub(crate) struct OverlayPipes {
    pub files: Vec<FileMeta<'static>>,
    pub readers: Vec<UnixStream>,
    pub handle: JoinHandle<Result<()>>,
}

impl OverlayPipes {
    pub(crate) async fn join(self) -> Result<()> {
        self.handle
            .await
            .map_err(|e| WizardError::BuildError(format!("join overlay task: {e}")))?
            .map_err(|e| WizardError::BuildError(format!("stream overlay components: {e}")))
    }
}

/// Writes an overlay file to its pipe, skipping paths not in the overlay.
fn write_overlay_file(
    path: &str,
    reader: &mut dyn std::io::Read,
    path_to_index: &HashMap<&str, usize>,
    writers: &mut [UnixStream],
) -> Result<()> {
    let Some(writer) = path_to_index
        .get(path)
        .and_then(|&idx| writers.get_mut(idx))
    else {
        return Ok(());
    };
    std::io::copy(reader, writer)
        .map_err(|e| WizardError::BuildError(format!("stream overlay file: {e}")))?;

    Ok(())
}

/// Sets up overlay file pipes and spawns a streaming task.
pub(crate) async fn setup(plan: &BuildPlan) -> Result<OverlayPipes> {
    let Some(ov) = plan.overlay() else {
        return Err(WizardError::BuildError(
            "overlay requested but none configured".to_owned(),
        ));
    };

    let files = overlay::metadata(ov).await?;
    let pipe_pairs: Vec<(UnixStream, UnixStream)> = files
        .iter()
        .map(|_| {
            UnixStream::pair()
                .map_err(|e| WizardError::BuildError(format!("create overlay pipe: {e}")))
        })
        .collect::<Result<Vec<_>>>()?;

    let readers: Vec<UnixStream> = pipe_pairs
        .iter()
        .map(|pair| {
            pair.0
                .try_clone()
                .map_err(|e| WizardError::BuildError(format!("clone overlay pipe: {e}")))
        })
        .collect::<Result<Vec<_>>>()?;

    let mut writers: Vec<UnixStream> = pipe_pairs.into_iter().map(|(_r, w)| w).collect();

    let path_to_index: HashMap<&str, usize> = files
        .iter()
        .enumerate()
        .map(|(i, meta)| (meta.path, i))
        .collect();

    let overlay_info = ov.clone();
    let handle = tokio::spawn(async move {
        overlay::pull(&overlay_info, |path, _size, reader| {
            write_overlay_file(path, reader, &path_to_index, &mut writers)
        })
        .await
    });

    Ok(OverlayPipes {
        files,
        readers,
        handle,
    })
}
