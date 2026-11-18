//! Release of the cgroup of a service that is known to be dead.
//!
//! The hierarchy itself is mounted once by `crate::cgroup`; this module only
//! touches the cgroup of one service, and only after its main process is gone.
//! That is where the cgroup earns its keep as a lifecycle tool: the main
//! process may have left children behind, and `cgroup.kill` reaches them
//! without walking the process table and guessing which of them belong to the
//! service.

use std::path::Path;
use std::time::Duration;

use sandbox::cgroup::v2::Tree;
use sandbox::error::SandboxError;

use crate::cgroup::ROOT;

// How many times the removal is retried while the kernel is still killing the
// tasks of the cgroup. The whole sequence is short on purpose: a cgroup left in
// place is recoverable, because the spawner claims it again, while a supervisor
// stopped for long is not.
const ATTEMPTS: u32 = 5;

// The delay between two removals, long enough for the kernel to finish killing
// a handful of tasks and short enough not to be felt by the event loop.
const DELAY: Duration = Duration::from_millis(20);

/// Kills every process left in the cgroup of `name` and removes it.
///
/// The main process of a service is already gone when this runs, but its
/// children are not necessarily, so the kill comes first and the removal after.
/// `cgroup.kill` returns before the tasks are gone, so the removal is retried a
/// bounded number of times and then abandoned with a warning, because a cgroup
/// that outlives its service is a leak the next spawn recovers from.
///
/// A cgroup that is not there is not an error: a spawn can fail before it
/// creates one, and an already removed cgroup is the outcome this asks for.
pub(crate) async fn release(name: &str) {
    let root = Path::new(ROOT);
    let tree = match Tree::open(root, name) {
        Ok(tree) => tree,
        // A spawn that failed before creating the cgroup leaves nothing to
        // release, which is not an error. The binding keeps its underscore
        // because the debug call compiles away without the `debug` feature.
        Err(_error) => {
            kmsg::debug!("Service {name} has no cgroup to release: {_error}");
            return;
        }
    };
    if let Err(error) = tree.kill() {
        kmsg::warn!("cgroup: killing the cgroup of {name} failed: {error}");
    }

    retire(root, name).await;
}

// Removes the directory, retrying while it is still populated: `rmdir` answers
// `EBUSY` until the tasks killed above are actually gone.
async fn retire(root: &Path, name: &str) {
    let mut last = String::new();
    for attempt in 1..=ATTEMPTS {
        match remove(root, name) {
            Ok(()) => return,
            Err(error) if gone(&error) => return,
            Err(error) => last = error.to_string(),
        }
        // The delay belongs before the next removal only: the last attempt is
        // followed by the warning, not by another try.
        if attempt < ATTEMPTS {
            tokio::time::sleep(DELAY).await;
        }
    }
    kmsg::warn!("cgroup: leaving the cgroup of {name} in place, still in use: {last}");
}

// Opens and removes the directory in one step, because `Tree::remove` consumes
// the tree it was called on and a retry needs a fresh one.
fn remove(root: &Path, name: &str) -> Result<(), SandboxError> {
    Tree::open(root, name)?.remove()
}

// Whether `error` reports that the directory is already absent, which is the
// outcome the caller wanted and not a reason to try again.
fn gone(error: &SandboxError) -> bool {
    matches!(error, SandboxError::Io(errno) if errno.raw() == rustix::io::Errno::NOENT.raw_os_error())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absent_directory_counts_as_gone() {
        // ARRANGE
        let absent = SandboxError::Io(sandbox::error::Errno::new(
            rustix::io::Errno::NOENT.raw_os_error(),
        ));
        let busy = SandboxError::Io(sandbox::error::Errno::new(
            rustix::io::Errno::BUSY.raw_os_error(),
        ));

        // ACT
        let absent_is_gone = gone(&absent);
        let busy_is_gone = gone(&busy);

        // ASSERT
        assert!(absent_is_gone, "a missing directory is already removed");
        assert!(!busy_is_gone, "a busy directory is worth retrying");
    }
}
