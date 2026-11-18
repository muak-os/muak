//! Integration tests for the cgroup v2 tree a workload is born in.

#![cfg(all(feature = "cgroupv2", feature = "spawn"))]

mod common;

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::process::ExitStatusExt as _;
    use std::path::Path;
    use std::sync::atomic::{AtomicU32, Ordering};

    use sandbox::cgroup::Limits;
    use sandbox::cgroup::v2::Tree;
    use sandbox::error::{Mechanism, SandboxError};
    use sandbox::policy::Policy;
    use sandbox::probe::Capabilities;
    use sandbox::spawn::{self, Child, Cmd};

    use crate::common;

    const ROOT: &str = "/sys/fs/cgroup";
    const SLEEP: &[&core::ffi::CStr] = &[c"/bin/sleep", c"30"];

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    #[test]
    fn every_requested_limit_lands_in_its_file() {
        // ARRANGE
        if !writable() {
            return;
        }
        let name = unique("limits");
        let tree = Tree::create(
            Path::new(ROOT),
            &name,
            &Limits::new().memory_max(64 * 1024 * 1024).pids_max(8),
        )
        .unwrap();
        let path = Path::new(ROOT).join(&name);

        // ACT
        let memory = read(&path.join("memory.max"));
        let pids = read(&path.join("pids.max"));
        let kill = path.join("cgroup.kill").exists();
        tree.remove().unwrap();

        // ASSERT
        assert_eq!(memory.trim(), "67108864", "memory.max differs");
        assert_eq!(pids.trim(), "8", "pids.max differs");
        assert!(kill, "the cgroup has no cgroup.kill");
    }

    #[test]
    fn a_child_is_born_inside_its_cgroup() {
        // ARRANGE
        if !writable() {
            return;
        }
        let name = unique("birth");
        let tree = Tree::create(Path::new(ROOT), &name, &Limits::new()).unwrap();
        let child = spawn_in(&tree);

        // ACT
        let membership = read(Path::new(&format!("/proc/{}/cgroup", child.pid())));

        // ASSERT
        tree.kill().unwrap();
        child.wait().unwrap();
        tree.remove().unwrap();
        let line = membership.trim();
        assert!(
            line.ends_with(&format!("/{name}")),
            "the child was not born in the cgroup: {line}"
        );
    }

    #[test]
    fn kill_terminates_a_child_sleeping_in_the_cgroup() {
        // ARRANGE
        if !writable() {
            return;
        }
        let name = unique("kill");
        let tree = Tree::create(Path::new(ROOT), &name, &Limits::new()).unwrap();
        let child = spawn_in(&tree);

        // ACT
        tree.kill().unwrap();
        let status = child.wait().unwrap();

        // ASSERT
        tree.remove().unwrap();
        assert_eq!(
            status.signal(),
            Some(libc::SIGKILL),
            "cgroup.kill did not kill the child"
        );
    }

    #[test]
    fn removing_a_populated_cgroup_fails() {
        // ARRANGE
        if !writable() {
            return;
        }
        let name = unique("populated");
        let path = Path::new(ROOT).join(&name);
        let tree = Tree::create(Path::new(ROOT), &name, &Limits::new()).unwrap();
        let child = spawn_in(&tree);

        // ACT
        let result = tree.remove();

        // ASSERT
        child.kill().unwrap();
        child.wait().unwrap();
        fs::remove_dir(&path).unwrap();
        assert!(
            result.is_err(),
            "removing a cgroup that still holds a process must fail"
        );
    }

    #[test]
    fn a_tree_cannot_be_created_with_an_invalid_name() {
        // ARRANGE
        let limits = Limits::new();

        // ACT
        let result = Tree::create(Path::new(ROOT), "../escape", &limits);

        // ASSERT
        assert!(
            matches!(
                result,
                Err(SandboxError::Unsupported {
                    mechanism: Mechanism::Cgroup,
                    ..
                })
            ),
            "an invalid name must be refused before the filesystem: {result:?}"
        );
    }

    // Spawns a sleeping child inside `tree`, which proves placement by birth
    // rather than by a write to `cgroup.procs` afterwards.
    fn spawn_in(tree: &Tree<'_>) -> Child {
        let policy = Policy::new().id(common::identity());
        let cmd = Cmd::new(c"/bin/sleep", SLEEP, &[]);
        let resolved = policy.resolve(&Capabilities::empty()).unwrap();
        let fd = tree.fd().unwrap();
        spawn::run(&resolved, &cmd, Some(fd)).unwrap()
    }

    fn read(path: &Path) -> String {
        match fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) => panic!("{} is unreadable: {error}", path.display()),
        }
    }

    fn unique(label: &str) -> String {
        let count = COUNTER.fetch_add(1, Ordering::SeqCst);
        format!("sandbox-{label}-{}-{count}", std::process::id())
    }

    // A test process can run as root and still sit in a container whose cgroup
    // hierarchy is read only or absent, which is a skip: the mechanism is not
    // there to be tested rather than broken.
    fn writable() -> bool {
        let probe = Path::new(ROOT).join(unique("probe"));
        match fs::create_dir_all(&probe) {
            Ok(()) => {
                let _unused = fs::remove_dir(&probe);
                true
            }
            Err(error) => {
                println!("skipping: {ROOT} does not accept a new cgroup: {error}");
                false
            }
        }
    }
}
