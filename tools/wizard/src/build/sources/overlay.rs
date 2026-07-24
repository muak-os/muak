use std::collections::HashMap;
use std::os::unix::net::UnixStream;

use esp::FileMeta;
use tar::{Builder, Header};
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
        return Ok(OverlayPipes {
            files: Vec::new(),
            readers: Vec::new(),
            handle: tokio::spawn(async { Ok(()) }),
        });
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

/// A pipe carrying a streaming tar archive of overlay files.
pub(crate) struct OverlayTar {
    pub reader: UnixStream,
    pub handle: JoinHandle<Result<()>>,
}

/// Spawns a background task that pulls overlay files from OCI and
/// streams them into a tar archive on a pipe. Returns the pipe's read
/// end and a join handle.
pub(crate) fn setup_tar(overlay: &overlay::Overlay) -> Result<OverlayTar> {
    let (reader, writer) = UnixStream::pair()
        .map_err(|e| WizardError::BuildError(format!("create overlay tar pipe: {e}")))?;

    let handle = tokio::spawn(pull_overlay_to_tar(overlay.clone(), writer));

    Ok(OverlayTar { reader, handle })
}

async fn pull_overlay_to_tar(ov: overlay::Overlay, writer: UnixStream) -> Result<()> {
    let mut builder = Builder::new(writer);
    overlay::pull(&ov, |path, size, reader| -> Result<()> {
        append_to_tar(&mut builder, path, size, reader)
    })
    .await?;
    builder
        .finish()
        .map_err(|e| WizardError::BuildError(format!("finish overlay tar: {e}")))?;

    Ok(())
}

fn append_to_tar(
    builder: &mut Builder<UnixStream>,
    path: &str,
    size: u64,
    reader: &mut dyn std::io::Read,
) -> Result<()> {
    let mut header = Header::new_gnu();
    header
        .set_path(path)
        .map_err(|e| WizardError::BuildError(format!("set tar header path: {e}")))?;
    header.set_size(size);
    header.set_mode(0o644);
    builder
        .append(&header, reader)
        .map_err(|e| WizardError::BuildError(format!("append to tar: {e}")))?;

    Ok(())
}
