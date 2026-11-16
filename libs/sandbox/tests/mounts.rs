//! Integration tests for mounts and the root filesystem switch.
//!
//! Both mechanisms need `CAP_SYS_ADMIN`, so every test prints why it skipped
//! when the test process cannot get it: it does not run as root, or it runs as
//! root in a container or in a capability dropping runner that took the
//! capability away, and the kernel then refuses the step.

#![cfg(all(
    feature = "spawn",
    feature = "probe",
    feature = "namespaces",
    feature = "mounts",
    feature = "rootfs"
))]

extern crate alloc;

mod common;

#[cfg(test)]
mod tests {
    use alloc::ffi::CString;
    use std::fs;
    use std::io::Read as _;
    use std::os::fd::AsFd as _;
    use std::os::unix::ffi::OsStrExt as _;
    use std::path::{Path, PathBuf};

    use sandbox::error::{Mechanism, SandboxError};
    use sandbox::mount::{Entry, Flags, Source};
    use sandbox::namespace::Set;
    use sandbox::policy::Policy;
    use sandbox::probe;
    use sandbox::resolve::Resolved;
    use sandbox::spawn::{self, Child, Cmd};

    use crate::common;

    #[test]
    fn a_mount_in_the_child_leaves_the_parent_mount_table_untouched() {
        // ARRANGE
        if !root() {
            println!("skipping: mounts need CAP_SYS_ADMIN");
            return;
        }
        let target = tempfile::tempdir().unwrap();
        let entries = [Entry {
            source: Source::Tmpfs,
            target: target.path(),
            data: None,
            flags: Flags::new(),
        }];
        let policy = Policy::new()
            .id(common::identity())
            .namespaces(Set::MOUNT)
            .mounts(&entries);
        let resolved = policy
            .resolve(&probe::current())
            .expect("the policy must resolve on this host");
        let argv = [c"/bin/cat", c"/proc/self/mountinfo"];
        let (mut reader, writer) = std::io::pipe().unwrap();
        let cmd = Cmd::new(c"/bin/cat", &argv, &[]).stdout(writer.as_fd());
        let before = mountinfo();

        // ACT
        let Some(child) = run_or_skip(&resolved, &cmd) else {
            return;
        };
        let status = child.wait().expect("the child must be waited on");
        drop(writer);
        let mut output = String::new();
        reader.read_to_string(&mut output).unwrap();
        let after = mountinfo();

        // ASSERT
        assert_eq!(status.code(), Some(0_i32), "the child failed: {status:?}");
        // The child prints its own mount table, so a step that returns `Ok`
        // without mounting anything is caught here rather than by the parent
        // comparison below, which holds for an empty child table too.
        let mount_point = target.path().to_str().unwrap();
        assert!(
            line_at(&output, mount_point).is_some(),
            "the child mounted nothing at {mount_point}: {output}"
        );
        assert!(
            before == after,
            "the mount of the child propagated into the parent mount table"
        );
    }

    #[test]
    fn the_root_switch_makes_the_tmpfs_root_the_root() {
        // ARRANGE
        if !root() {
            println!("skipping: the root switch needs CAP_SYS_ADMIN");
            return;
        }
        let work = tempfile::tempdir().unwrap();
        let root = work.path().join("root");
        fs::create_dir_all(&root).unwrap();
        // The tmpfs is mounted here, in the parent, and its directories are
        // created below it. A mount point inside a fresh tmpfs does not exist
        // yet and the child path never creates directories, so the tree has to
        // be built before the child runs.
        assert!(mount_tmpfs(&root), "cannot mount the test tmpfs");
        // The mount belongs to the host namespace, so it is detached when this
        // test returns whether it passes or panics.
        let _teardown = Teardown(&root);
        let names: Vec<&str> = ["bin", "sbin", "usr", "lib", "lib64", "lib32"]
            .into_iter()
            .filter(|name| Path::new("/").join(name).exists())
            .collect();
        let bind = names.first().expect("the host has no directory to bind");
        let sources: Vec<PathBuf> = names.iter().map(|name| Path::new("/").join(name)).collect();
        let targets: Vec<PathBuf> = names.iter().map(|name| root.join(name)).collect();
        for target in &targets {
            fs::create_dir_all(target).unwrap();
        }
        let proc_target = root.join("proc");
        fs::create_dir_all(&proc_target).unwrap();
        // A writable bind, so the secure flags of the default `Flags` are
        // checked next to the read only ones.
        let etc = root.join("etc");
        fs::create_dir_all(&etc).unwrap();

        let mut entries: Vec<Entry<'_>> = Vec::with_capacity(sources.len() + 2);
        entries.push(Entry {
            source: Source::Proc,
            target: &proc_target,
            data: None,
            flags: Flags::new(),
        });
        entries.push(Entry {
            source: Source::Bind(Path::new("/etc")),
            target: &etc,
            data: None,
            flags: Flags::new(),
        });
        for (source, target) in sources.iter().zip(&targets) {
            entries.push(Entry {
                source: Source::BindReadOnly(source),
                target,
                data: None,
                // The workload is read from the host, so this one bind has to
                // carry the one flag the secure default refuses.
                flags: Flags {
                    no_exec: false,
                    ..Flags::new()
                },
            });
        }
        let policy = Policy::new()
            .id(common::identity())
            .namespaces(Set::MOUNT)
            .mounts(&entries)
            .rootfs(&root);
        let resolved = policy
            .resolve(&probe::current())
            .expect("the policy must resolve on this host");
        let argv = [c"/bin/cat", c"/proc/self/mountinfo"];
        let (mut reader, writer) = std::io::pipe().unwrap();
        let cmd = Cmd::new(c"/bin/cat", &argv, &[]).stdout(writer.as_fd());

        // ACT
        let Some(child) = run_or_skip(&resolved, &cmd) else {
            return;
        };
        let status = child.wait().expect("the child must be waited on");
        drop(writer);
        let mut output = String::new();
        reader.read_to_string(&mut output).unwrap();

        // ASSERT
        assert_eq!(status.code(), Some(0_i32), "the child failed: {status:?}");
        let root_line = line_at(&output, "/").expect("the new root must appear in the mount table");
        assert!(
            root_line.contains(" - tmpfs "),
            "the new root is not the tmpfs: {root_line}"
        );
        let mount_point = format!("/{bind}");
        let read_only = line_at(&output, &mount_point)
            .unwrap_or_else(|| panic!("the bind of {mount_point} is missing: {output}"));
        assert!(
            carries_options(read_only, &["ro"]),
            "the read only bind of {mount_point} is writable: {read_only}"
        );
        let writable = line_at(&output, "/etc")
            .unwrap_or_else(|| panic!("the bind of /etc is missing: {output}"));
        assert!(
            carries_options(writable, &["nosuid", "nodev", "noexec"]),
            "the default bind lost its secure flags: {writable}"
        );
    }

    fn root() -> bool {
        // SAFETY: `geteuid` reads process state and dereferences no pointer.
        unsafe { libc::geteuid() == 0 }
    }

    // Runs `cmd` under `resolved`, skipping the test with a printed reason when
    // the kernel refuses the namespace or mount step. Running as root is not
    // enough to carry `CAP_SYS_ADMIN`, so a refusal there is a skip and not a
    // failure.
    fn run_or_skip(resolved: &Resolved<'_>, cmd: &Cmd<'_>) -> Option<Child> {
        match spawn::run(resolved, cmd) {
            Ok(child) => Some(child),
            Err(error) if refused(&error) => {
                println!("skipping: the kernel refused the sandbox: {error}");
                None
            }
            Err(error) => panic!("the child must spawn: {error:?}"),
        }
    }

    // Returns whether `error` is the kernel refusing a step the environment did
    // not grant, which is a skip, rather than a policy the kernel accepted.
    fn refused(error: &SandboxError) -> bool {
        matches!(
            error,
            SandboxError::Denied {
                mechanism: Mechanism::Namespaces | Mechanism::Mounts,
                source,
            } if matches!(source.raw(), libc::EPERM | libc::EACCES)
        )
    }

    fn mountinfo() -> Vec<u8> {
        fs::read("/proc/self/mountinfo").unwrap()
    }

    // Returns the mountinfo line whose mount point is `mount_point`.
    fn line_at<'a>(table: &'a str, mount_point: &str) -> Option<&'a str> {
        table
            .lines()
            .find(|line| line.split_whitespace().nth(4) == Some(mount_point))
    }

    // Returns whether the per mount options of a mountinfo line carry every
    // option named. The superblock options after the separator are ignored,
    // because a read only bind of a writable filesystem keeps them writable.
    fn carries_options(line: &str, wanted: &[&str]) -> bool {
        let options: Vec<&str> = line
            .split(" - ")
            .next()
            .unwrap_or_default()
            .split_whitespace()
            .skip(5)
            .flat_map(|field| field.split(','))
            .collect();
        wanted.iter().all(|option| options.contains(option))
    }

    fn mount_tmpfs(target: &Path) -> bool {
        let target = c_string(target);
        // SAFETY: `mount` reads three NUL terminated strings that outlive the
        // call and two scalar arguments, and it touches no memory of ours.
        unsafe {
            libc::mount(
                c"tmpfs".as_ptr(),
                target.as_ptr(),
                c"tmpfs".as_ptr(),
                0,
                core::ptr::null(),
            ) == 0
        }
    }

    fn detach(target: &Path) {
        let target = c_string(target);
        // SAFETY: `umount2` reads one NUL terminated string that outlives the
        // call and one scalar flag, and it touches no memory of ours.
        let _status = unsafe { libc::umount2(target.as_ptr(), libc::MNT_DETACH) };
    }

    fn c_string(path: &Path) -> CString {
        CString::new(path.as_os_str().as_bytes()).unwrap()
    }

    // Detaches the tmpfs a test mounted in the host namespace when the test
    // returns, so a panicking assertion cannot leave it in the host table.
    struct Teardown<'a>(&'a Path);

    impl Drop for Teardown<'_> {
        fn drop(&mut self) {
            detach(self.0);
        }
    }
}
