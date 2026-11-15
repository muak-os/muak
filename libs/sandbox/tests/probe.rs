//! Integration tests for the kernel capability set and its detection.

#![cfg(feature = "probe")]

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use sandbox::probe::{self, Capabilities, detect};
    use tempfile::TempDir;

    const ALL: [Capabilities; 27] = [
        Capabilities::MOUNT_NS,
        Capabilities::PID_NS,
        Capabilities::NET_NS,
        Capabilities::IPC_NS,
        Capabilities::UTS_NS,
        Capabilities::USER_NS,
        Capabilities::CGROUP_NS,
        Capabilities::TIME_NS,
        Capabilities::CGROUP_V2,
        Capabilities::OVERLAYFS,
        Capabilities::TMPFS,
        Capabilities::DEVPTS,
        Capabilities::PROC,
        Capabilities::SYSFS,
        Capabilities::EROFS,
        Capabilities::SECCOMP,
        Capabilities::SECCOMP_NOTIF,
        Capabilities::PIVOT_ROOT,
        Capabilities::OPEN_TREE,
        Capabilities::MOUNT_SETATTR,
        Capabilities::IDMAPPED_MOUNTS,
        Capabilities::CAPSET,
        Capabilities::CAP_BOUNDING,
        Capabilities::PIDFD,
        Capabilities::CLONE3,
        Capabilities::CLONE_INTO_CGROUP,
        Capabilities::UNPRIV_USER_NS,
    ];

    #[test]
    fn every_flag_is_a_distinct_single_bit() {
        // ARRANGE
        let mut bits = [0_u32; 27];

        // ACT
        for (position, flag) in ALL.into_iter().enumerate() {
            bits[position] = flag.bits();
        }

        // ASSERT
        for (position, value) in bits.into_iter().enumerate() {
            assert_eq!(value.count_ones(), 1, "flag {position} is not a single bit");
            assert!(
                !bits[..position].contains(&value),
                "flag {position} repeats bit {value:#x}"
            );
        }
    }

    #[test]
    fn all_flags_fit_in_the_bit_set() {
        // ARRANGE
        let mut union = Capabilities::empty();

        // ACT
        for flag in ALL {
            union = union.insert(flag);
        }

        // ASSERT
        assert_eq!(union.bits(), 0x2fff_f9ff);
    }

    #[test]
    fn empty_set_ignores_every_flag() {
        // ARRANGE
        let set = Capabilities::empty();
        let flag = Capabilities::UNPRIV_USER_NS;

        // ACT
        let contained = set.contains(flag);
        let intersecting = set.intersects(flag);
        let empty = set.is_empty();

        // ASSERT
        assert!(!contained);
        assert!(!intersecting);
        assert!(empty);
        assert!(set.contains(Capabilities::empty()));
    }

    #[test]
    fn union_tracks_what_was_inserted() {
        // ARRANGE
        let set = Capabilities::MOUNT_NS.insert(Capabilities::SECCOMP);

        // ACT
        let both = set.contains(Capabilities::MOUNT_NS.insert(Capabilities::SECCOMP));
        let has_mount = set.intersects(Capabilities::MOUNT_NS);
        let has_cgroup = set.intersects(Capabilities::CGROUP_V2);
        let subset = set.contains(Capabilities::MOUNT_NS);
        let missing = set.contains(Capabilities::MOUNT_NS.insert(Capabilities::CGROUP_V2));
        let empty = set.is_empty();

        // ASSERT
        assert!(both);
        assert!(has_mount);
        assert!(!has_cgroup);
        assert!(subset);
        assert!(!missing);
        assert!(!empty);
    }

    #[test]
    fn display_of_empty_set_is_none() {
        // ARRANGE
        let set = Capabilities::empty();

        // ACT
        let displayed = set.to_string();

        // ASSERT
        assert_eq!(displayed, "none");
    }

    #[test]
    fn display_lists_every_set_bit() {
        // ARRANGE
        let set = Capabilities::MOUNT_NS.insert(Capabilities::PID_NS);

        // ACT
        let displayed = set.to_string();

        // ASSERT
        assert_eq!(displayed, "MOUNT_NS, PID_NS");
    }

    #[test]
    fn namespaces_follow_their_entries() {
        // ARRANGE
        let root = fixture();
        for name in ["mnt", "pid", "net", "ipc", "uts", "cgroup", "time"] {
            touch(root.path(), &format!("proc/self/ns/{name}"));
        }

        // ACT
        let caps = detect::files(root.path());

        // ASSERT
        assert_eq!(
            caps,
            Capabilities::MOUNT_NS
                .insert(Capabilities::PID_NS)
                .insert(Capabilities::NET_NS)
                .insert(Capabilities::IPC_NS)
                .insert(Capabilities::UTS_NS)
                .insert(Capabilities::CGROUP_NS)
                .insert(Capabilities::TIME_NS)
        );
    }

    #[test]
    fn zero_namespace_limit_clears_the_flag() {
        // ARRANGE
        let root = fixture();
        touch(root.path(), "proc/self/ns/mnt");
        touch(root.path(), "proc/self/ns/pid");
        text(root.path(), "proc/sys/user/max_mnt_namespaces", "0\n");

        // ACT
        let caps = detect::files(root.path());

        // ASSERT
        assert_eq!(caps, Capabilities::PID_NS);
    }

    #[test]
    fn user_namespaces_follow_the_limit_and_the_clone_sysctl() {
        // ARRANGE
        let mainline = fixture();
        text(
            mainline.path(),
            "proc/sys/user/max_user_namespaces",
            "1500\n",
        );
        let patched = fixture();
        text(
            patched.path(),
            "proc/sys/user/max_user_namespaces",
            "1500\n",
        );
        text(
            patched.path(),
            "proc/sys/kernel/unprivileged_userns_clone",
            "0\n",
        );
        let blocked = fixture();
        text(blocked.path(), "proc/sys/user/max_user_namespaces", "0\n");
        text(
            blocked.path(),
            "proc/sys/kernel/unprivileged_userns_clone",
            "1\n",
        );

        // ACT
        let mainline_caps = detect::files(mainline.path());
        let patched_caps = detect::files(patched.path());
        let blocked_caps = detect::files(blocked.path());

        // ASSERT
        assert_eq!(
            mainline_caps,
            Capabilities::USER_NS.insert(Capabilities::UNPRIV_USER_NS)
        );
        assert_eq!(patched_caps, Capabilities::USER_NS);
        assert!(blocked_caps.is_empty(), "detected {blocked_caps}");
    }

    #[test]
    fn cgroup_controllers_describe_v2() {
        // ARRANGE
        let root = fixture();
        text(
            root.path(),
            "sys/fs/cgroup/cgroup.controllers",
            "cpu memory\n",
        );

        // ACT
        let caps = detect::files(root.path());

        // ASSERT
        assert_eq!(caps, Capabilities::CGROUP_V2);
    }

    #[test]
    fn filesystems_accept_a_nodev_prefix() {
        // ARRANGE
        let root = fixture();
        text(
            root.path(),
            "proc/filesystems",
            "nodev\toverlay\nnodev tmpfs\nnodev\tdevpts\n\tproc\nsysfs\nerofs\n\text4\n",
        );

        // ACT
        let caps = detect::files(root.path());

        // ASSERT
        assert_eq!(
            caps,
            Capabilities::OVERLAYFS
                .insert(Capabilities::TMPFS)
                .insert(Capabilities::DEVPTS)
                .insert(Capabilities::PROC)
                .insert(Capabilities::SYSFS)
                .insert(Capabilities::EROFS)
        );
    }

    #[test]
    fn seccomp_actions_are_read_from_the_list() {
        // ARRANGE
        let with_notif = fixture();
        text(
            with_notif.path(),
            "proc/sys/kernel/seccomp/actions_avail",
            "kill_process errno user_notif log allow\n",
        );
        let without_notif = fixture();
        text(
            without_notif.path(),
            "proc/sys/kernel/seccomp/actions_avail",
            "kill_process errno log allow\n",
        );

        // ACT
        let notif = detect::files(with_notif.path());
        let plain = detect::files(without_notif.path());

        // ASSERT
        assert_eq!(
            notif,
            Capabilities::SECCOMP.insert(Capabilities::SECCOMP_NOTIF)
        );
        assert_eq!(plain, Capabilities::SECCOMP);
    }

    #[test]
    fn current_host_reports_the_namespaces_it_always_has() {
        // ARRANGE
        let expected = Capabilities::MOUNT_NS
            .insert(Capabilities::PID_NS)
            .insert(Capabilities::IPC_NS)
            .insert(Capabilities::UTS_NS)
            .insert(Capabilities::CGROUP_NS);

        // ACT
        let caps = probe::current();

        // ASSERT
        assert!(caps.contains(expected), "running host reported {caps}");
    }

    #[test]
    fn empty_root_yields_nothing() {
        // ARRANGE
        let root = fixture();

        // ACT
        let caps = detect::files(root.path());

        // ASSERT
        assert!(caps.is_empty(), "detected {caps}");
    }

    #[test]
    fn unreadable_files_leave_their_flags_unset() {
        // ARRANGE
        let root = fixture();
        directory(root.path(), "proc/filesystems");
        directory(root.path(), "proc/sys/kernel/seccomp/actions_avail");
        directory(root.path(), "sys/fs/cgroup/cgroup.controllers");

        // ACT
        let caps = detect::files(root.path());

        // ASSERT
        assert!(caps.is_empty(), "detected {caps}");
    }

    fn fixture() -> TempDir {
        tempfile::tempdir().unwrap()
    }

    fn text(root: &Path, relative: &str, contents: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    fn touch(root: &Path, relative: &str) {
        text(root, relative, "");
    }

    fn directory(root: &Path, relative: &str) {
        fs::create_dir_all(root.join(relative)).unwrap();
    }
}
