//! Release of the cgroup of a service that is known to be dead.

use std::path::Path;
use std::time::Duration;

use sandbox::cgroup::v2::Tree;
use sandbox::error::SandboxError;

use crate::cgroup::ROOT;

// How many times the removal is retried while the kernel is still killing tasks: a leftover cgroup is recoverable, because the spawner claims it again.
const ATTEMPTS: u32 = 5;

// The delay between two removals, short enough not to be felt by the event loop.
const DELAY: Duration = Duration::from_millis(20);

/// Kills every process left in the cgroup of `name` and removes it.
pub(crate) async fn release(name: &str) {
    let root = Path::new(ROOT);
    let tree = match Tree::open(root, name) {
        Ok(tree) => tree,
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

// `rmdir` answers `EBUSY` until the tasks killed above are actually gone.
async fn retire(root: &Path, name: &str) {
    let mut last = String::new();
    for attempt in 1..=ATTEMPTS {
        match remove(root, name) {
            Ok(()) => return,
            Err(error) if gone(&error) => return,
            Err(error) => last = error.to_string(),
        }
        if attempt < ATTEMPTS {
            tokio::time::sleep(DELAY).await;
        }
    }
    kmsg::warn!("cgroup: leaving the cgroup of {name} in place, still in use: {last}");
}

// Opens and removes in one step, because `Tree::remove` consumes the tree and a retry needs a fresh one.
fn remove(root: &Path, name: &str) -> Result<(), SandboxError> {
    Tree::open(root, name)?.remove()
}

// Whether the directory is already absent, which is the outcome the caller wanted.
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
