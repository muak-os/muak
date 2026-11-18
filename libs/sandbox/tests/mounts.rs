//! Integration tests for mounts and the root filesystem switch.
//!
//! Both mechanisms need `CAP_SYS_ADMIN`, so every test prints why it skipped when the capability is not granted.

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

    use rustix::{mount, thread};
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
        // A propagation test needs a shared peer group to propagate into, and the inherited `/` may be private.
        if !in_shared_namespace() {
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
        let Some(child) = run_or_skip(resolved, &cmd) else {
            return;
        };
        let status = child.wait().expect("the child must be waited on");
        drop(writer);
        let mut output = String::new();
        reader.read_to_string(&mut output).unwrap();
        let after = mountinfo();

        // ASSERT
        assert_eq!(status.code(), Some(0_i32), "the child failed: {status:?}");
        // The child prints its own mount table, so a step that returns `Ok` without mounting anything is caught here.
        let mount_point = target.path().to_str().unwrap();
        assert!(
            line_at(&output, mount_point).is_some(),
            "the child mounted nothing at {mount_point}: {output}"
        );
        // A leak of this mount can only surface at its own path, because a test running beside this one mounts elsewhere.
        assert!(
            line_at(&before, mount_point).is_none(),
            "{mount_point} was already mounted before the child ran"
        );
        let leaked: Vec<&str> = after
            .lines()
            .filter(|line| at_or_below(mount_point_of(line), mount_point))
            .collect();
        assert!(
            leaked.is_empty(),
            "the mount of the child propagated into the parent mount table: {leaked:?}"
        );
    }

    #[test]
    fn the_root_switch_makes_the_tmpfs_root_the_root() {
        // ARRANGE
        if !root() {
            println!("skipping: the root switch needs CAP_SYS_ADMIN");
            return;
        }
        // This test mounts in the parent namespace, so it first moves into a private one: on a shared `/` those mounts would reach the host table.
        if !in_private_namespace() {
            return;
        }
        let work = tempfile::tempdir().unwrap();
        let root = work.path().join("root");
        fs::create_dir_all(&root).unwrap();
        // The directories are created below the tmpfs here, because a mount point inside a fresh tmpfs does not exist and the child path never creates directories.
        assert!(mount_tmpfs(&root), "cannot mount the test tmpfs");
        // The mount dies with the namespace, and the guard detaches it anyway so the failure path leaves nothing behind.
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
        // A writable bind, so the secure flags of the default `Flags` are checked next to the read only ones.
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
                // The workload is read from the host, so this one bind carries the flag the secure default refuses.
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
        let Some(child) = run_or_skip(resolved, &cmd) else {
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

    // Runs `cmd` under `resolved`, skipping the test when the kernel refuses the namespace or mount step.
    fn run_or_skip(resolved: Resolved<'_>, cmd: &Cmd<'_>) -> Option<Child> {
        match spawn::run(&resolved, cmd, None) {
            Ok(child) => Some(child),
            Err(error) if refused(&error) => {
                println!("skipping: the kernel refused the sandbox: {error}");
                None
            }
            Err(error) => panic!("the child must spawn: {error:?}"),
        }
    }

    // Returns whether `error` is the kernel refusing a step the environment did not grant, which is a skip.
    fn refused(error: &SandboxError) -> bool {
        matches!(
            error,
            SandboxError::Denied {
                mechanism: Mechanism::Namespaces | Mechanism::Mounts,
                source,
            } if matches!(source.raw(), libc::EPERM | libc::EACCES)
        )
    }

    // Moves the calling thread into a private mount namespace, so nothing this test mounts reaches the table it started in.
    fn in_private_namespace() -> bool {
        // SAFETY: the mount namespace alone is unshared, so no file descriptor table is unshared under a thread that may still use it.
        let created =
            unsafe { thread::unshare_unsafe(thread::UnshareFlags::NEWNS) }.and_then(|()| {
                mount::mount_change(
                    "/",
                    mount::MountPropagationFlags::REC | mount::MountPropagationFlags::PRIVATE,
                )
            });
        match created {
            Ok(()) => true,
            Err(error) => {
                println!("skipping: the mount namespace cannot be made private: {error}");
                false
            }
        }
    }

    // Moves the calling thread into a mount namespace whose `/` is a shared peer group of its own, so a child mount propagates into this table and nowhere else.
    fn in_shared_namespace() -> bool {
        if !in_private_namespace() {
            return false;
        }
        // Private first, shared second: sharing the host `/` would hand the host's peer group a new peer.
        match mount::mount_change(
            "/",
            mount::MountPropagationFlags::REC | mount::MountPropagationFlags::SHARED,
        ) {
            Ok(()) => true,
            Err(error) => {
                println!("skipping: the mount namespace cannot be made shared: {error}");
                false
            }
        }
    }

    // Reads the mount table of the calling thread: the namespace a test unshares belongs to that thread, while `/proc/self` points at the thread group leader.
    fn mountinfo() -> String {
        String::from_utf8_lossy(&fs::read("/proc/thread-self/mountinfo").unwrap()).into_owned()
    }

    // Returns the mount point, the fifth field, of a mountinfo line.
    fn mount_point_of(line: &str) -> &str {
        line.split_whitespace().nth(4).unwrap_or_default()
    }

    // Returns the mountinfo line whose mount point is `mount_point`.
    fn line_at<'a>(table: &'a str, mount_point: &str) -> Option<&'a str> {
        table
            .lines()
            .find(|line| mount_point_of(line) == mount_point)
    }

    // Returns whether `point` is `target` or a path below it.
    fn at_or_below(point: &str, target: &str) -> bool {
        point
            .strip_prefix(target)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
    }

    // Returns whether the per mount options of a mountinfo line carry every option named, ignoring the superblock options after the separator.
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
        // SAFETY: `mount` reads three NUL terminated strings that outlive the call and two scalar arguments, and it touches no memory of ours.
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
        // SAFETY: `umount2` reads one NUL terminated string that outlives the call and one scalar flag, and it touches no memory of ours.
        let _status = unsafe { libc::umount2(target.as_ptr(), libc::MNT_DETACH) };
    }

    fn c_string(path: &Path) -> CString {
        CString::new(path.as_os_str().as_bytes()).unwrap()
    }

    // Detaches the tmpfs a test mounted when the test returns, so a panicking assertion cannot leave it behind.
    struct Teardown<'a>(&'a Path);

    impl Drop for Teardown<'_> {
        fn drop(&mut self) {
            detach(self.0);
        }
    }
}
