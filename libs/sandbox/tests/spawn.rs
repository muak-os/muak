//! Integration tests for spawning under a policy.

#![cfg(feature = "spawn")]

mod common;

#[cfg(test)]
mod tests {
    use std::io::Read as _;
    use std::os::fd::AsFd as _;
    use std::os::unix::process::ExitStatusExt as _;

    use sandbox::error::{Mechanism, Result, SandboxError};
    use sandbox::policy::Policy;
    use sandbox::probe::Capabilities;
    use sandbox::spawn::{self, Child, Cmd, Signal};

    use crate::common;

    const ENOENT: i32 = 2;
    const EBADF: i32 = 9;

    fn spawn_under(policy: &Policy<'_>, cmd: &Cmd<'_>) -> Result<Child> {
        let resolved = policy.resolve(&Capabilities::empty())?;
        spawn::run(&resolved, cmd)
    }

    fn identity() -> Policy<'static> {
        Policy::new().id(common::identity())
    }

    #[test]
    fn successful_run_reports_the_exit_status() {
        // ARRANGE
        let policy = identity();
        let argv = [c"/bin/sh", c"-c", c"exit 7"];
        let cmd = Cmd::new(c"/bin/sh", &argv, &[]);

        // ACT
        let child = spawn_under(&policy, &cmd).unwrap();
        let status = child.wait().unwrap();

        // ASSERT
        assert_eq!(status.code(), Some(7_i32));
    }

    #[test]
    fn missing_program_is_denied_at_exec() {
        // ARRANGE
        let policy = identity();
        let argv = [c"/nonexistent/sandbox/program"];
        let cmd = Cmd::new(c"/nonexistent/sandbox/program", &argv, &[]);

        // ACT
        let result = spawn_under(&policy, &cmd);

        // ASSERT
        match result {
            Err(SandboxError::Denied {
                mechanism: Mechanism::Spawn,
                source,
            }) => assert_eq!(source.raw(), ENOENT),
            other => panic!("expected a denied exec, got {other:?}"),
        }
    }

    #[test]
    fn stdout_is_captured_through_a_pipe() {
        // ARRANGE
        let policy = identity();
        let argv = [c"/bin/sh", c"-c", c"printf captured"];
        let (mut reader, writer) = std::io::pipe().unwrap();
        let mut output = String::new();

        // ACT
        let status = {
            let cmd = Cmd::new(c"/bin/sh", &argv, &[]).stdout(writer.as_fd());
            let child = spawn_under(&policy, &cmd).unwrap();
            child.wait().unwrap()
        };
        drop(writer);
        reader.read_to_string(&mut output).unwrap();

        // ASSERT
        assert_eq!(status.code(), Some(0_i32));
        assert_eq!(output, "captured");
    }

    #[test]
    fn passed_descriptor_reaches_the_child() {
        // ARRANGE
        let policy = identity();
        let argv = [c"/bin/sh", c"-c", c"printf passed >&3"];
        let (mut reader, writer) = std::io::pipe().unwrap();
        let pass = [(writer.as_fd(), 3_i32)];
        let mut output = String::new();

        // ACT
        let status = {
            let cmd = Cmd::new(c"/bin/sh", &argv, &[]).pass(&pass);
            let child = spawn_under(&policy, &cmd).unwrap();
            child.wait().unwrap()
        };
        drop(writer);
        reader.read_to_string(&mut output).unwrap();

        // ASSERT
        assert_eq!(status.code(), Some(0_i32));
        assert_eq!(output, "passed");
    }

    #[test]
    fn failing_pass_is_denied_at_exec() {
        // ARRANGE
        let policy = identity();
        let argv = [c"/bin/true"];
        let (_reader, writer) = std::io::pipe().unwrap();
        let pass = [(writer.as_fd(), -1_i32)];
        let cmd = Cmd::new(c"/bin/true", &argv, &[]).pass(&pass);

        // ACT
        let result = spawn_under(&policy, &cmd);

        // ASSERT
        match result {
            Err(SandboxError::Denied {
                mechanism: Mechanism::Spawn,
                source,
            }) => assert_eq!(source.raw(), EBADF),
            other => panic!("expected a denied exec, got {other:?}"),
        }
    }

    #[test]
    fn pidfd_is_opened_when_the_kernel_reports_it() {
        // ARRANGE
        let policy = identity();
        let argv = [c"/bin/sh", c"-c", c"exit 0"];
        let cmd = Cmd::new(c"/bin/sh", &argv, &[]);
        let resolved = policy.resolve(&Capabilities::PIDFD).unwrap();

        // ACT
        let child = spawn::run(&resolved, &cmd).unwrap();
        let status = child.wait().unwrap();

        // ASSERT
        assert_eq!(status.code(), Some(0_i32));
        assert!(child.pidfd().is_some(), "the child has no pidfd");
    }

    #[test]
    fn killing_a_child_is_reported_as_a_signal() {
        // ARRANGE
        let policy = identity();
        let argv = [c"/bin/sh", c"-c", c"sleep 30"];
        let cmd = Cmd::new(c"/bin/sh", &argv, &[]);
        let child = spawn_under(&policy, &cmd).unwrap();

        // ACT
        child.signal(Signal::KILL).unwrap();
        let status = child.wait().unwrap();

        // ASSERT
        assert_eq!(status.signal(), Some(libc::SIGKILL));
    }

    #[test]
    fn wait_survives_a_signal_storm() {
        // ARRANGE
        let policy = identity();
        let argv = [c"/bin/sh", c"-c", c"sleep 1"];
        let cmd = Cmd::new(c"/bin/sh", &argv, &[]);
        let child = spawn_under(&policy, &cmd).unwrap();
        install_handler();

        // ACT
        let storm = std::thread::spawn(storm);
        let status = child.wait();
        storm.join().unwrap();

        // ASSERT
        assert_eq!(status.unwrap().code(), Some(0_i32));
    }

    #[test]
    fn eight_threads_spawn_twenty_children_each() {
        // ARRANGE
        let threads: Vec<_> = std::iter::repeat_with(|| std::thread::spawn(spawn_twenty))
            .take(8)
            .collect();

        // ACT
        let outcomes: Vec<_> = threads
            .into_iter()
            .map(std::thread::JoinHandle::join)
            .collect();

        // ASSERT
        for outcome in outcomes {
            assert!(outcome.is_ok(), "a spawning thread panicked");
        }
    }

    extern "C" fn quiet(_signal: libc::c_int) {}

    fn storm() {
        for _ in 0_i32..5_000_i32 {
            // SAFETY: `getpid` reads process state and takes no pointer.
            let pid = unsafe { libc::getpid() };
            // SAFETY: `kill` takes only scalar arguments, so no pointer is
            // involved, and the signal targets this process only.
            unsafe {
                libc::kill(pid, libc::SIGUSR1);
            }
        }
    }

    fn spawn_twenty() {
        for _ in 0_i32..20_i32 {
            let policy = identity();
            let argv = [c"/bin/sh", c"-c", c"exit 0"];
            let cmd = Cmd::new(c"/bin/sh", &argv, &[]);
            let child = spawn_under(&policy, &cmd).unwrap();
            assert_eq!(child.wait().unwrap().code(), Some(0_i32));
        }
    }

    fn install_handler() {
        // SAFETY: an all-zero `sigaction` is a valid value.
        let mut action: libc::sigaction = unsafe { core::mem::zeroed() };
        let handler: *const () = core::ptr::from_ref(&quiet).cast();
        action.sa_sigaction = handler.addr();
        // SAFETY: the mask is an initialized field of the zeroed struct, and
        // `sigemptyset` writes only to it.
        unsafe {
            libc::sigemptyset(&raw mut action.sa_mask);
        }
        // SAFETY: `action` is initialized and outlives the call; a null old
        // action asks the kernel to skip the previous handler.
        let result =
            unsafe { libc::sigaction(libc::SIGUSR1, &raw const action, core::ptr::null_mut()) };
        assert_eq!(result, 0_i32);
    }
}
