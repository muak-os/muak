//! Single-writer, bounded and crash-consistent local state.

use std::fs::{self, File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde::Serialize;
use serde::de::DeserializeOwned;
use uuid::Uuid;

const MAX_STATE_BYTES: u64 = 1024 * 1024;

pub(crate) struct Store<T> {
    path: PathBuf,
    _lock: File,
    data: T,
    healthy: bool,
}

impl<T: Serialize + DeserializeOwned> Store<T> {
    pub(crate) fn open(path: &Path, initial: impl FnOnce() -> Result<T>) -> Result<Self> {
        let parent = path
            .parent()
            .context("state file needs a parent directory")?;
        fs::create_dir_all(parent)?;
        if let Some(ancestor) = parent
            .parent()
            .filter(|ancestor| !ancestor.as_os_str().is_empty())
        {
            File::open(ancestor)?.sync_all()?;
        }
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .open(path.with_extension("lock"))?;
        lock.try_lock()
            .context("fleet state is already owned by another process")?;
        let existing = match File::open(path) {
            Ok(file) => {
                let mut bytes = Vec::new();
                file.take(MAX_STATE_BYTES + 1).read_to_end(&mut bytes)?;
                Some(decode(&bytes)?)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        let missing = existing.is_none();
        let mut store = Self {
            path: path.to_owned(),
            _lock: lock,
            data: match existing {
                Some(data) => data,
                None => initial()?,
            },
            healthy: true,
        };
        if missing {
            store.write_current()?;
        }
        Ok(store)
    }

    pub(crate) fn data(&self) -> Result<&T> {
        if !self.healthy {
            bail!("fleet storage has an uncertain outcome; restart and recover before continuing");
        }
        Ok(&self.data)
    }

    pub(crate) fn commit(&mut self, next: T) -> Result<()> {
        self.data()?;
        // Any failed durable transition poisons this instance, including rename
        // succeeding before directory fsync fails. Never continue on old policy.
        self.healthy = false;
        replace(&self.path, &next)?;
        self.data = next;
        self.healthy = true;
        Ok(())
    }

    fn write_current(&mut self) -> Result<()> {
        replace(&self.path, &self.data)
    }
}

fn replace(path: &Path, data: &impl Serialize) -> Result<()> {
    let bytes = serde_json::to_vec(data)?;
    if u64::try_from(bytes.len())? > MAX_STATE_BYTES {
        bail!("fleet state exceeds size limit");
    }
    let parent = path.parent().context("state file needs a parent")?;
    let temporary = parent.join(format!(".fleet-{}.tmp", Uuid::new_v4()));
    let outcome = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if outcome.is_err() {
        let _cleanup = fs::remove_file(&temporary);
    }
    outcome
}

fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    if u64::try_from(bytes.len())? > MAX_STATE_BYTES {
        bail!("fleet state exceeds size limit");
    }
    serde_json::from_slice(bytes).context("invalid fleet state; refusing empty recovery")
}
