//! Integration tests for credentials and no-new-privileges.

#![cfg(feature = "spawn")]

mod common;

#[cfg(test)]
mod tests {
    use std::os::unix::fs::MetadataExt as _;

    use sandbox::credentials::Id;
    use sandbox::policy::Policy;
    use sandbox::probe::Capabilities;
    use sandbox::spawn::{self, Cmd};

    use crate::common;

    #[test]
    fn no_new_privs_is_visible_in_the_child_status() {
        // ARRANGE
        let policy = Policy::new().id(common::identity()).no_new_privs();
        let argv = [
            c"/bin/sh",
            c"-c",
            c"grep -q '^NoNewPrivs:[[:space:]]*1' /proc/self/status",
        ];
        let cmd = Cmd::new(c"/bin/sh", &argv, &[]);
        let resolved = policy.resolve(&Capabilities::empty()).unwrap();

        // ACT
        let child = spawn::run(&resolved, &cmd).unwrap();
        let status = child.wait().unwrap();

        // ASSERT
        assert_eq!(
            status.code(),
            Some(0_i32),
            "the child did not see NoNewPrivs 1"
        );
    }

    #[test]
    fn dropping_to_a_non_root_uid_reaches_the_child() {
        // ARRANGE
        let Ok(metadata) = std::fs::metadata("/proc/self") else {
            panic!("/proc/self must describe the running test process");
        };
        if metadata.uid() != 0 {
            eprintln!("skipping credential drop test: it needs root to set another uid");
            return;
        }
        let policy = Policy::new().id(Id::new(0xFFFE, 0xFFFE));
        let argv = [c"/bin/sh", c"-c", c"test \"$(id -u)\" = 65534"];
        let cmd = Cmd::new(c"/bin/sh", &argv, &[]);
        let resolved = policy.resolve(&Capabilities::empty()).unwrap();

        // ACT
        let child = spawn::run(&resolved, &cmd).unwrap();
        let status = child.wait().unwrap();

        // ASSERT
        assert_eq!(status.code(), Some(0_i32), "the child kept the root uid");
    }
}
