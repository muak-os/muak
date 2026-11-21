//! Filesystem work on the unified cgroup v2 hierarchy.

use core::cell::OnceCell;
use std::os::fd::{AsFd as _, BorrowedFd, OwnedFd};
use std::path::{Path, PathBuf};

use rustix::fs::{self, Mode, OFlags};
use rustix::io;
use rustix::process::Pid;

use crate::cgroup::Limits;
use crate::error::{Errno, Mechanism, Result, SandboxError};

// The mode a fresh cgroup directory gets: writable by its owner only.
const MODE: u32 = 0o755;
const KILL: &str = "cgroup.kill";
const PROCS: &str = "cgroup.procs";

/// One cgroup below a root, borrowed from the caller's root path and name.
#[derive(Debug)]
pub struct Tree<'a> {
    root: &'a Path,
    name: &'a str,
    fd: OnceCell<OwnedFd>,
}

impl<'a> Tree<'a> {
    /// Creates the cgroup `name` below `root` and writes `limits` into it.
    ///
    /// # Errors
    ///
    /// Returns [`SandboxError::Unsupported`] when the name is invalid or the new cgroup carries no `cgroup.kill` file, and [`SandboxError::Io`] when the directory or a limit file cannot be written.
    pub fn create(root: &'a Path, name: &'a str, limits: &Limits) -> Result<Self> {
        let tree = checked(root, name)?;
        let path = tree.path();
        fs::mkdir(&path, Mode::from_bits_truncate(MODE)).map_err(failed)?;
        for (file, contents) in limits.rendered().into_iter().flatten() {
            write(&path.join(file), &contents)?;
        }
        if fs::stat(path.join(KILL)).is_err() {
            return Err(SandboxError::Unsupported {
                mechanism: Mechanism::Cgroup,
                detail: "cgroup.kill is missing from the new cgroup",
            });
        }
        Ok(tree)
    }

    /// Creates the cgroup `name` below `root`, clearing a leftover directory first.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`Tree::create`] and those of clearing a leftover, including the `EBUSY` a caller must retry later.
    pub fn claim(root: &'a Path, name: &'a str, limits: &Limits) -> Result<Self> {
        match Self::create(root, name, limits) {
            Ok(tree) => Ok(tree),
            Err(error) if exists(&error) => {
                let leftover = Self::open(root, name)?;
                leftover.kill()?;
                leftover.remove()?;
                Self::create(root, name, limits)
            }
            Err(error) => Err(error),
        }
    }

    /// Opens the cgroup `name` below `root`, which must already exist.
    ///
    /// # Errors
    ///
    /// Returns [`SandboxError::Unsupported`] when the name is invalid, and [`SandboxError::Io`] when the directory is missing or is not a directory.
    pub fn open(root: &'a Path, name: &'a str) -> Result<Self> {
        let tree = checked(root, name)?;
        let _descriptor = tree.fd()?;

        Ok(tree)
    }

    /// Places `pid` in this cgroup by writing it to `cgroup.procs`.
    ///
    /// # Errors
    ///
    /// Returns [`SandboxError::Io`] when `cgroup.procs` cannot be written.
    pub fn place(&self, pid: Pid) -> Result<()> {
        write(&self.path().join(PROCS), &pid.to_string())
    }

    /// Kills every process in this cgroup by writing `1` to `cgroup.kill`.
    ///
    /// # Errors
    ///
    /// Returns [`SandboxError::Io`] when `cgroup.kill` cannot be written.
    pub fn kill(&self) -> Result<()> {
        write(&self.path().join(KILL), "1")
    }

    /// Removes the cgroup directory.
    ///
    /// # Errors
    ///
    /// Returns [`SandboxError::Io`] when the directory cannot be removed, including a cgroup that is still populated.
    pub fn remove(self) -> Result<()> {
        let Self {
            root,
            name,
            fd: cache,
        } = self;
        drop(cache);

        fs::rmdir(root.join(name)).map_err(failed)
    }

    /// Returns a descriptor for this cgroup, opening it on first use.
    ///
    /// # Errors
    ///
    /// Returns [`SandboxError::Io`] when the directory cannot be opened.
    pub fn fd(&self) -> Result<BorrowedFd<'_>> {
        if let Some(cached) = self.fd.get() {
            return Ok(cached.as_fd());
        }
        let opened = self.descriptor()?;

        Ok(self.fd.get_or_init(|| opened).as_fd())
    }

    fn path(&self) -> PathBuf {
        self.root.join(self.name)
    }

    fn descriptor(&self) -> Result<OwnedFd> {
        let flags = OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY;

        fs::open(self.path(), flags, Mode::empty()).map_err(failed)
    }
}

fn checked<'a>(root: &'a Path, name: &'a str) -> Result<Tree<'a>> {
    if !super::valid_name(name) {
        return Err(SandboxError::Unsupported {
            mechanism: Mechanism::Cgroup,
            detail: "use a name that is neither empty, . nor .., holds no / and is shorter than 256 bytes",
        });
    }

    Ok(Tree {
        root,
        name,
        fd: OnceCell::new(),
    })
}

const fn exists(error: &SandboxError) -> bool {
    matches!(error, SandboxError::Io(errno) if errno.raw() == io::Errno::EXIST.raw_os_error())
}

fn failed(error: io::Errno) -> SandboxError {
    SandboxError::Io(Errno::from(error))
}

fn write(path: &Path, contents: &str) -> Result<()> {
    let fd = fs::open(path, OFlags::WRONLY | OFlags::CLOEXEC, Mode::empty()).map_err(failed)?;
    let mut rest = contents.as_bytes();
    while !rest.is_empty() {
        let written = io::write(&fd, rest).map_err(failed)?;
        rest = rest.get(written..).unwrap_or_default();
    }

    Ok(())
}
