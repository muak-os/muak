//! The child path between creation and exec. No allocations allowed.

use core::ffi::c_char;
use std::os::fd::{AsRawFd as _, BorrowedFd, RawFd};

use crate::error::Errno;
#[cfg(feature = "mounts")]
use crate::mount;
#[cfg(feature = "namespaces")]
use crate::namespace;
use crate::policy::Policy;
#[cfg(feature = "rootfs")]
use crate::rootfs;
use crate::stage::Stage;
#[cfg(feature = "limits")]
use crate::sys::linux::limits;
use crate::sys::linux::{credentials, spawn};

type Failure = (Stage, Errno);

/// # Safety
///
/// Must be called in the child of a successful `clone3` whose parent built `program`, `argv` and `envp` and left the descriptors open. It never returns.
pub(crate) unsafe fn exec(
    policy: &Policy<'_>,
    program: *const c_char,
    argv: *const *const c_char,
    envp: *const *const c_char,
    files: [RawFd; 3],
    pass: &[(BorrowedFd<'_>, RawFd)],
    report: RawFd,
) -> ! {
    if let Err((stage, errno)) = apply(policy) {
        fail(report, stage, errno);
    }

    let targets = [libc::STDIN_FILENO, libc::STDOUT_FILENO, libc::STDERR_FILENO];
    for (source, target) in files.into_iter().zip(targets) {
        if source >= 0_i32
            && let Err(errno) = spawn::redirect(source, target)
        {
            fail(report, Stage::Exec, errno);
        }
    }

    for &(source, target) in pass {
        if let Err(errno) = spawn::redirect(source.as_raw_fd(), target) {
            fail(report, Stage::Exec, errno);
        }
        if let Err(errno) = spawn::clear_cloexec(target) {
            fail(report, Stage::Exec, errno);
        }
    }

    if let Err(errno) = spawn::restore_signals() {
        fail(report, Stage::Exec, errno);
    }

    // SAFETY: the pointers come from a `Cmd` the parent built and kept alive across the fork, so they name NUL terminated strings and null terminated arrays in this copy of the address space.
    let errno = unsafe { spawn::exec(program, argv, envp) };

    fail(report, Stage::Exec, errno);
}

fn apply(policy: &Policy<'_>) -> Result<(), Failure> {
    trap(Stage::Signals, spawn::reset_signals())?;

    #[cfg(feature = "namespaces")]
    if !policy.namespaces.is_empty() {
        trap(Stage::Namespaces, namespace::apply(policy.namespaces))?;
    }

    #[cfg(feature = "mounts")]
    if !policy.mounts.is_empty() || policy.rootfs.is_some() {
        trap(Stage::Mounts, mount::apply(policy.mounts))?;
    }

    #[cfg(feature = "rootfs")]
    if let Some(root) = policy.rootfs {
        trap(Stage::Rootfs, rootfs::apply(root))?;
    }

    #[cfg(feature = "limits")]
    trap(Stage::Limits, limits::apply(&policy.limits))?;

    #[cfg(feature = "caps")]
    if let Some(kept) = policy.caps {
        trap(Stage::Caps, credentials::bounding(kept))?;
    }

    if let Some(groups) = policy.id.groups {
        trap(Stage::Credentials, credentials::groups(groups))?;
    }
    trap(Stage::Credentials, credentials::gid(policy.id.gid))?;
    trap(Stage::Credentials, credentials::uid(policy.id.uid))?;

    #[cfg(feature = "caps")]
    if let Some(kept) = policy.caps {
        trap(Stage::Caps, credentials::effective(kept))?;
    }

    if policy.no_new_privs {
        trap(Stage::NoNewPrivs, credentials::no_new_privs())?;
    }

    Ok(())
}

fn trap(stage: Stage, result: Result<(), Errno>) -> Result<(), Failure> {
    result.map_err(|errno| (stage, errno))
}

fn fail(report: RawFd, stage: Stage, errno: Errno) -> ! {
    let packed = u64::from(stage.code())
        | (u64::from(u32::from_le_bytes(errno.raw().to_le_bytes())) << 16_u32);
    let bytes = packed.to_le_bytes();
    let mut written = 0_usize;
    while written < bytes.len() {
        let remaining = bytes.len().saturating_sub(written);
        // SAFETY: the offset stays strictly below the eight byte array length, so the pointer remains inside the array.
        let pointer = unsafe { bytes.as_ptr().add(written) };
        // SAFETY: `pointer` points to `remaining` initialized bytes of the array above.
        let result = unsafe { libc::write(report, pointer.cast(), remaining) };
        if result < 0 && !interrupted() {
            break;
        }
        if result > 0 {
            written = written.saturating_add(usize::try_from(result).unwrap_or(0));
        }
    }

    // SAFETY: `_exit` has no pointer preconditions, and 127 fits the integer argument the kernel expects.
    unsafe { libc::_exit(127) }
}

fn interrupted() -> bool {
    // SAFETY: `__errno_location` returns the current thread's errno slot, always a valid readable pointer.
    let location = unsafe { libc::__errno_location() };

    // SAFETY: `location` is the valid errno slot returned above.
    unsafe { *location == libc::EINTR }
}

#[cfg(test)]
mod tests {
    use core::mem::size_of;
    use core::sync::atomic::{AtomicUsize, Ordering};
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::fs::File;
    use std::io::Read as _;
    use std::os::fd::OwnedFd;

    use super::*;
    use crate::credentials::{Caps, Id};
    #[cfg(all(feature = "mounts", feature = "namespaces"))]
    use crate::mount::{Entry, Flags, Source};
    #[cfg(all(feature = "mounts", feature = "namespaces"))]
    use crate::namespace::Set;

    static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

    #[global_allocator]
    static GLOBAL: Counting = Counting;

    struct Counting;

    // SAFETY: every method forwards to the system allocator with the same layout it received, and the counter update is an atomic that cannot allocate or unwind.
    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
            // SAFETY: the layout is passed through unchanged to the system allocator, which honours the same contract.
            unsafe { System.alloc(layout) }
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            // SAFETY: the pointer and layout came from this allocator, which forwards them unchanged to the system allocator.
            unsafe { System.dealloc(ptr, layout) }
        }
    }

    fn reported_stage(policy: &Policy<'_>) -> Option<Stage> {
        let argv = [
            c"/nonexistent/sandbox/program".as_ptr(),
            core::ptr::null::<c_char>(),
        ];
        let (report, _output) = run(policy, &argv);

        reported(&report).map(|(stage, _errno)| stage)
    }

    // Runs the child path for `policy` in a forked child and returns what it reported plus what `argv` wrote to standard output. An empty report means the child reached the exec.
    fn run(policy: &Policy<'_>, argv: &[*const c_char]) -> (Vec<u8>, Vec<u8>) {
        let (read, write) = spawn::pipe().unwrap();
        let report_fd = write.as_raw_fd();
        let (output, out_write) = spawn::pipe().unwrap();
        let stdout_fd = out_write.as_raw_fd();
        let envp = [core::ptr::null::<c_char>()];
        let files = [-1_i32, stdout_fd, -1_i32];
        // SAFETY: the child runs the child path against the pointers built above, which stay valid in its copy of the address space, and `exec` never returns.
        let pid = unsafe { libc::fork() };
        if pid == 0_i32 {
            // SAFETY: the conditions above hold in the forked child, whose only work is this call.
            unsafe {
                exec(
                    policy,
                    argv[0],
                    argv.as_ptr(),
                    envp.as_ptr(),
                    files,
                    &[],
                    report_fd,
                );
            }
        }
        drop(write);
        drop(out_write);
        let report = read_all(read);
        let output = read_all(output);
        let mut status = 0_i32;
        // SAFETY: `pid` is the child forked above and `status` is a valid out pointer.
        unsafe {
            libc::waitpid(pid, &raw mut status, 0);
        };

        (report, output)
    }

    fn reported(bytes: &[u8]) -> Option<(Stage, Errno)> {
        if bytes.is_empty() {
            return None;
        }
        let packed = u64::from_le_bytes(bytes.try_into().ok()?);
        let code = u16::try_from(packed & 0xffff).unwrap_or(0);
        let stage = Stage::from_code(code)?;
        let raw = u32::try_from((packed >> 16_u32) & 0xffff_ffff).unwrap_or(0);

        Some((stage, Errno::new(i32::from_le_bytes(raw.to_le_bytes()))))
    }

    fn read_all(fd: OwnedFd) -> Vec<u8> {
        let mut bytes = Vec::new();
        File::from(fd).read_to_end(&mut bytes).unwrap();

        bytes
    }

    #[cfg(all(feature = "mounts", feature = "namespaces"))]
    fn refused(outcome: Option<(Stage, Errno)>) -> bool {
        let Some((stage, errno)) = outcome else {
            return false;
        };
        let denied = matches!(stage, Stage::Namespaces | Stage::Mounts)
            && matches!(errno.raw(), libc::EPERM | libc::EACCES);
        if denied {
            println!("skipping: the kernel refused the {stage:?} step: {errno}");
        }

        denied
    }

    fn identity() -> Id<'static> {
        // SAFETY: `geteuid` and `getegid` read process state and dereference no pointer.
        let uid = unsafe { libc::geteuid() };
        // SAFETY: as above.
        let gid = unsafe { libc::getegid() };

        Id::new(uid, gid)
    }

    #[cfg(all(feature = "mounts", feature = "namespaces"))]
    fn runs_as_root() -> bool {
        // SAFETY: `geteuid` reads process state and dereferences no pointer.
        unsafe { libc::geteuid() == 0 }
    }

    #[test]
    fn pre_exec_half_does_not_allocate() {
        // ARRANGE
        // SAFETY: `geteuid` and `getegid` read process state and dereference no pointer.
        let uid = unsafe { libc::geteuid() };
        // SAFETY: `getegid` reads process state and dereferences no pointer.
        let gid = unsafe { libc::getegid() };
        // The kernel validates the count and not the gids, so only an unprivileged run reaches the `setgroups` failure path.
        let groups = [u32::MAX];
        let policy = Policy::new()
            .id(Id::new(uid, gid).groups(&groups))
            .caps(Caps::all());
        let (read, write) = spawn::pipe().unwrap();
        let write_fd = write.as_raw_fd();

        // ACT
        // SAFETY: the test child runs only `apply`, which the crate keeps allocation free and lock free, then writes a fixed integer and exits.
        let pid = unsafe { libc::fork() };
        if pid == 0_i32 {
            let before = ALLOCATIONS.load(Ordering::SeqCst);
            let outcome = apply(&policy);
            let after = ALLOCATIONS.load(Ordering::SeqCst);
            let delta = after.saturating_sub(before);
            let bytes = delta.to_le_bytes();
            // SAFETY: `bytes` is initialized and outlives the call, and the write targets the pipe the parent still reads.
            unsafe {
                libc::write(write_fd, bytes.as_ptr().cast(), bytes.len());
            }
            // SAFETY: the child must not run destructors that belong to the parent, so it leaves through `_exit`.
            unsafe {
                libc::_exit(i32::from(outcome.is_err()));
            };
        }
        drop(write);
        let mut bytes = [0_u8; size_of::<usize>()];
        let mut report = File::from(read);
        report.read_exact(&mut bytes).unwrap();
        let mut status = 0_i32;
        // SAFETY: `pid` is the child forked above and `status` is a valid out pointer.
        unsafe {
            libc::waitpid(pid, &raw mut status, 0);
        };
        let delta = usize::from_le_bytes(bytes);

        // ASSERT
        assert_eq!(delta, 0, "the pre-exec half made {delta} allocations");
    }

    #[cfg(all(feature = "mounts", feature = "namespaces"))]
    #[test]
    fn the_pre_exec_half_reaches_the_mount_step_before_exec() {
        // ARRANGE
        if !runs_as_root() {
            println!("skipping: the namespace and mount steps need CAP_SYS_ADMIN");
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("absent");
        let entries = [Entry {
            source: Source::Tmpfs,
            target: &target,
            data: None,
            flags: Flags::new(),
        }];
        let policy = Policy::new()
            .id(identity())
            .namespaces(Set::MOUNT)
            .mounts(&entries);
        let argv = [c"/bin/true".as_ptr(), core::ptr::null::<c_char>()];

        // ACT
        let (report, _output) = run(&policy, &argv);
        let outcome = reported(&report);

        // ASSERT
        if refused(outcome) {
            return;
        }
        assert_eq!(
            outcome,
            Some((Stage::Mounts, Errno::new(libc::ENOENT))),
            "the child did not reach the mount step"
        );
    }

    #[cfg(all(feature = "mounts", feature = "namespaces"))]
    #[test]
    fn the_pre_exec_half_mounts_every_entry_before_exec() {
        // ARRANGE
        if !runs_as_root() {
            println!("skipping: the namespace and mount steps need CAP_SYS_ADMIN");
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path();
        let entries = [Entry {
            source: Source::Tmpfs,
            target,
            data: None,
            flags: Flags::new(),
        }];
        let policy = Policy::new()
            .id(identity())
            .namespaces(Set::MOUNT)
            .mounts(&entries);
        let argv = [
            c"/bin/cat".as_ptr(),
            c"/proc/self/mountinfo".as_ptr(),
            core::ptr::null::<c_char>(),
        ];

        // ACT
        let (report, output) = run(&policy, &argv);
        let outcome = reported(&report);

        // ASSERT
        if refused(outcome) {
            return;
        }
        assert_eq!(outcome, None, "the child stopped before exec");
        let output = String::from_utf8(output).unwrap();
        let mount_point = target.to_str().unwrap();
        assert!(
            output
                .lines()
                .any(|line| line.split_whitespace().nth(4) == Some(mount_point)),
            "the tmpfs is missing from the child mount table: {output}"
        );
    }

    #[test]
    fn the_pre_exec_half_reaches_exec_without_namespaces_or_mounts() {
        // ARRANGE
        let policy = Policy::new().id(identity());

        // ACT
        let stage = reported_stage(&policy);

        // ASSERT
        assert_eq!(
            stage,
            Some(Stage::Exec),
            "an empty mount list and namespace set stopped the child at {stage:?}"
        );
    }
}
