//! Mounts a child performs before exec.
//!
//! The types always compile, because a policy must always be expressible. The
//! sequence that applies them, `apply`, needs `mounts` and `spawn`: the child
//! path between fork and exec is its only caller, and [`crate::sys::linux`]
//! carries the calls it makes.
//!
//! Every mount here is made inside the child's own mount namespace, which the
//! resolution step refuses to skip: a mount made in the host's namespace
//! propagates into the host mount table and can leave the host unusable.
//!
//! # Out of scope
//!
//! Overlay mounts belong to the phase that assembles image layers.
//!
//! Mount teardown is absent on purpose. The kernel destroys a mount namespace
//! when the last process in it exits, so every mount this module makes
//! disappears with the child and the parent has nothing to unmount.

use core::ffi::CStr;
use std::path::Path;

#[cfg(all(feature = "mounts", feature = "spawn"))]
use crate::error::Errno;
#[cfg(all(feature = "mounts", feature = "spawn"))]
use crate::sys::linux;

/// Filesystem a mount entry brings into the child.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source<'a> {
    /// Bind mount of an existing path.
    Bind(&'a Path),
    /// Bind mount of an existing path, read only whatever the source allows.
    BindReadOnly(&'a Path),
    /// Empty in memory filesystem, the usual sandbox root.
    Tmpfs,
    /// Kernel process table.
    Proc,
    /// Kernel device and bus tree.
    Sysfs,
    /// Pseudo terminal multiplexer.
    Devpts,
}

/// One mount the child performs before exec.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry<'a> {
    /// What is mounted.
    pub source: Source<'a>,
    /// Where it is mounted, resolved in the child after the namespaces exist.
    pub target: &'a Path,
    /// Option string the filesystem understands, ignored by bind mounts.
    pub data: Option<&'a CStr>,
    /// Flags the new mount carries.
    pub flags: Flags,
}

/// Flags a mount carries.
///
/// The defaults are the secure ones: `no_suid`, `no_dev` and `no_exec` are set
/// and `read_only` is not. A caller that has not thought about a flag therefore
/// gets the restrictive value and has to ask for the permissive one.
///
/// A bind mount applies these in a second syscall, because the kernel ignores
/// every flag but the recursive one in the call that creates the bind. A caller
/// that assumes one call is enough gets a writable, executable bind while
/// believing the opposite, which is why the child path pays for the remount.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Flags {
    /// Mount read only.
    pub read_only: bool,
    /// Ignore set-user-ID and set-group-ID bits.
    pub no_suid: bool,
    /// Refuse device nodes.
    pub no_dev: bool,
    /// Refuse execution.
    pub no_exec: bool,
    /// Apply the mount to every submount of the source or target.
    ///
    /// Only bind mounts honour this. A filesystem mount brings a new filesystem
    /// and has no submounts below it, so the kernel ignores the flag there and
    /// the filesystem path does not set it.
    pub recursive: bool,
}

impl Flags {
    /// Returns the secure default flags.
    #[must_use = "the flags are the value the caller asked for"]
    pub const fn new() -> Self {
        Self {
            read_only: false,
            no_suid: true,
            no_dev: true,
            no_exec: true,
            recursive: true,
        }
    }
}

impl Default for Flags {
    fn default() -> Self {
        Self::new()
    }
}

/// Makes the mount namespace private, then mounts every entry in order.
#[cfg(all(feature = "mounts", feature = "spawn"))]
pub(crate) fn apply(entries: &[Entry<'_>]) -> Result<(), Errno> {
    // Every mount below would otherwise propagate into the host mount
    // namespace, because the child inherits a shared mount tree from the host
    // and a shared mount forwards what is mounted under it to its peers. A
    // workload that mounts over `/` would then hide the host root for the whole
    // machine. The change is made first, even with no entries, so the caller
    // cannot forget it.
    linux::mount::private_root()?;
    entries.iter().try_for_each(entry)
}

#[cfg(all(feature = "mounts", feature = "spawn"))]
fn entry(entry: &Entry<'_>) -> Result<(), Errno> {
    match entry.source {
        Source::Bind(source) => linux::mount::bind(source, entry.target, entry.flags),
        Source::BindReadOnly(source) => {
            // The variant is what says read only, so the bit is forced here
            // rather than read from the flags the entry carries.
            let flags = Flags {
                read_only: true,
                ..entry.flags
            };
            linux::mount::bind(source, entry.target, flags)
        }
        Source::Tmpfs => linux::mount::filesystem(c"tmpfs", entry.target, entry.data, entry.flags),
        Source::Proc => linux::mount::filesystem(c"proc", entry.target, entry.data, entry.flags),
        Source::Sysfs => linux::mount::filesystem(c"sysfs", entry.target, entry.data, entry.flags),
        Source::Devpts => {
            linux::mount::filesystem(c"devpts", entry.target, entry.data, entry.flags)
        }
    }
}
