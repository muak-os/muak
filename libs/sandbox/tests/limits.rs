//! Integration tests for resource limits reaching the child.

#![cfg(all(feature = "spawn", feature = "limits"))]

mod common;

#[cfg(test)]
mod tests {
    use sandbox::limits::{Limit, Limits};
    use sandbox::policy::Policy;
    use sandbox::probe::Capabilities;
    use sandbox::spawn::{self, Cmd};

    use crate::common;

    #[test]
    fn nofile_limit_reaches_the_child() {
        // ARRANGE
        let policy = Policy::new()
            .id(common::identity())
            .limits(Limits::new().nofile(Limit::Value(64)));
        let argv = [c"/bin/sh", c"-c", c"test \"$(ulimit -n)\" = 64"];
        let cmd = Cmd::new(c"/bin/sh", &argv, &[]);
        let resolved = policy.resolve(&Capabilities::empty()).unwrap();

        // ACT
        let child = spawn::run(&resolved, &cmd).unwrap();
        let status = child.wait().unwrap();

        // ASSERT
        assert_eq!(
            status.code(),
            Some(0),
            "the child did not observe the open file limit"
        );
    }
}
