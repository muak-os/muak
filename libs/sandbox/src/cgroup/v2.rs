//! Filesystem work on the unified cgroup v2 hierarchy.
//!
//! Every call here runs on the parent side, so it may allocate and format. The
//! child path never reaches this module: the child is born inside its cgroup
//! through `CLONE_INTO_CGROUP`, which takes the directory descriptor this
//! module opens, so nothing is written after the child exists.
//!
//! `cgroup.kill` is required, never probed for a fallback. The kernel creates
//! it on every non root cgroup from 5.14 onwards, and a modern kernel always
//! satisfies that, so a cgroup without it is an error and not a reason to fall
//! back to signalling every pid in `cgroup.procs`.

use core::cell::OnceCell;
use std::os::fd::{AsFd as _, BorrowedFd, OwnedFd};
use std::path::{Path, PathBuf};

use rustix::fs::{self, Mode, OFlags};
use rustix::io;
use rustix::process::Pid;

use crate::cgroup::Limits;
use crate::error::{Errno, Mechanism, Result, SandboxError};

// The mode a fresh cgroup directory gets: readable by every process that may
// list the hierarchy, writable by its owner only.
const MODE: u32 = 0o755;

// The file the kernel reads as "kill every process in this cgroup".
const KILL: &str = "cgroup.kill";

// The file a process is written to when it is placed after it started.
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
    /// Returns [`SandboxError::Unsupported`] when `name` is not a valid cgroup
    /// name or when the new cgroup carries no `cgroup.kill` file, which the
    /// kernel only omits on a cgroup this crate cannot use, and
    /// [`SandboxError::Io`] when the directory or a limit file cannot be
    /// written.
    pub fn create(root: &'a Path, name: &'a str, limits: &Limits) -> Result<Self> {
        if !super::valid_name(name) {
            return Err(SandboxError::Unsupported {
                mechanism: Mechanism::Cgroup,
                detail: "use a name that is neither empty, . nor .., holds no / and is shorter than 256 bytes",
            });
        }
        let tree = Self {
            root,
            name,
            fd: OnceCell::new(),
        };
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

    /// Places `pid` in this cgroup by writing it to `cgroup.procs`.
    ///
    /// This exists for a caller that cannot use `CLONE_INTO_CGROUP`, and it is
    /// the worse of the two: the process runs from its creation until the write
    /// outside the cgroup it is about to be placed in. Use
    /// [`Tree::fd`] with `CLONE_INTO_CGROUP` instead whenever the child is
    /// created by this crate.
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
    /// The removal fails when the cgroup still holds a process, which is the
    /// point: a silent success there would hide a leaked process that the
    /// caller believes it killed.
    ///
    /// # Errors
    ///
    /// Returns [`SandboxError::Io`] when the directory cannot be removed,
    /// including the case of a cgroup that is still populated.
    pub fn remove(self) -> Result<()> {
        let Self {
            root,
            name,
            fd: cache,
        } = self;
        // The descriptor is dropped before the directory it names, so a caller
        // that never called `fd` costs nothing and one that did cannot hold the
        // directory open across the removal.
        drop(cache);
        fs::rmdir(root.join(name)).map_err(failed)
    }

    /// Returns a descriptor for this cgroup, opening it on first use.
    ///
    /// `CLONE_INTO_CGROUP` takes a directory descriptor, and the descriptor is
    /// cached so repeated spawns into the same cgroup do not reopen it. It is
    /// opened `O_CLOEXEC`, because no process should inherit a descriptor for a
    /// cgroup it already belongs to.
    ///
    /// # Errors
    ///
    /// Returns [`SandboxError::Io`] when the directory cannot be opened.
    pub fn fd(&self) -> Result<BorrowedFd<'_>> {
        if let Some(cached) = self.fd.get() {
            return Ok(cached.as_fd());
        }
        let opened = self.open()?;
        // A caller that lost a race with another one drops its own descriptor
        // here and shares the cached one instead.
        Ok(self.fd.get_or_init(|| opened).as_fd())
    }

    fn path(&self) -> PathBuf {
        self.root.join(self.name)
    }

    fn open(&self) -> Result<OwnedFd> {
        let flags = OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY;
        fs::open(self.path(), flags, Mode::empty()).map_err(failed)
    }
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
