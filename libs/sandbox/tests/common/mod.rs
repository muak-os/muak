//! Helpers shared by the spawn, credentials and limits integration tests.

use sandbox::credentials::Id;

/// Returns the effective identity of the test process.
///
/// A policy that leaves the default root identity would ask an unprivileged
/// test to call `setresuid(0)`, which fails. Spawning as the current identity
/// keeps the test focused on the mechanism it exercises.
#[must_use = "the identity is the value the test policy needs"]
pub(crate) fn identity() -> Id<'static> {
    // SAFETY: `geteuid` reads process state and dereferences no pointer.
    let uid = unsafe { libc::geteuid() };
    // SAFETY: `getegid` reads process state and dereferences no pointer.
    let gid = unsafe { libc::getegid() };
    Id::new(uid, gid)
}
