//! Credential, capability and no-new-privs syscalls.
//!
//! The calls are split by feature: dropping the identity is part of every
//! spawned child, while the capability set only exists when the `caps` feature
//! is compiled in. Each call reports the raw kernel number so the child can
//! write it through the report pipe without formatting anything.

use core::ffi::c_int;

#[cfg(feature = "caps")]
use crate::cap::Cap;
#[cfg(feature = "caps")]
use crate::credentials::Caps;
use crate::error::Errno;

#[cfg(feature = "caps")]
const CAPABILITY_VERSION_3: u32 = 0x2008_0522;

#[cfg(feature = "caps")]
#[repr(C)]
struct CapHeader {
    version: u32,
    pid: c_int,
}

#[cfg(feature = "caps")]
#[repr(C)]
struct CapData {
    effective: u32,
    permitted: u32,
    inheritable: u32,
}

pub(crate) fn groups(groups: &[u32]) -> Result<(), Errno> {
    // SAFETY: the slice owns `groups.len()` readable `gid_t` values and stays
    // borrowed for the duration of the call, so the kernel cannot read past it
    // or observe a freed buffer. An empty slice is passed with a zero count,
    // which clears every supplementary group without reading the pointer.
    let result = unsafe { libc::setgroups(groups.len(), groups.as_ptr()) };
    check(result)
}

pub(crate) fn gid(gid: u32) -> Result<(), Errno> {
    // SAFETY: `setresgid` only reads its scalar arguments and never dereferences
    // a pointer, so there is nothing to keep alive or validate.
    let result = unsafe { libc::setresgid(gid, gid, gid) };
    check(result)
}

pub(crate) fn uid(uid: u32) -> Result<(), Errno> {
    // SAFETY: `setresuid` only reads its scalar arguments and never dereferences
    // a pointer, so there is nothing to keep alive or validate.
    let result = unsafe { libc::setresuid(uid, uid, uid) };
    check(result)
}

pub(crate) fn no_new_privs() -> Result<(), Errno> {
    // SAFETY: the call reads only the scalar arguments and enables a process
    // wide flag, so no pointer is involved and nothing can be invalidated.
    let result = unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };
    check(result)
}

#[cfg(feature = "caps")]
pub(crate) fn bounding(kept: Caps) -> Result<(), Errno> {
    for cap in Cap::ALL {
        if kept.contains(cap) {
            continue;
        }
        // SAFETY: dropping a bounding capability reads only the scalar
        // capability number and never dereferences a pointer.
        let number = libc::c_ulong::from(cap.number());
        let result = unsafe { libc::prctl(libc::PR_CAPBSET_DROP, number, 0, 0, 0) };
        check(result)?;
    }
    Ok(())
}

#[cfg(feature = "caps")]
pub(crate) fn effective(kept: Caps) -> Result<(), Errno> {
    let bits = kept.bits();
    let low = u32::try_from(bits & u64::from(u32::MAX)).unwrap_or(u32::MAX);
    let high = u32::try_from(bits >> 32).unwrap_or(u32::MAX);
    let header = CapHeader {
        version: CAPABILITY_VERSION_3,
        pid: 0,
    };
    let data = [
        CapData {
            effective: low,
            permitted: low,
            inheritable: low,
        },
        CapData {
            effective: high,
            permitted: high,
            inheritable: high,
        },
    ];
    // SAFETY: both pointers refer to fully initialized `repr(C)` values that
    // outlive the call, and the header version asks the kernel for exactly the
    // two `u32` words the data array carries, so no element is read past it.
    let result = unsafe { libc::syscall(libc::SYS_capset, &header, data.as_ptr()) };
    check_i64(result)
}

fn check(result: c_int) -> Result<(), Errno> {
    if result == 0 { Ok(()) } else { Err(last()) }
}

#[cfg(feature = "caps")]
fn check_i64(result: libc::c_long) -> Result<(), Errno> {
    if result == 0 { Ok(()) } else { Err(last()) }
}

fn last() -> Errno {
    // SAFETY: `__errno_location` returns the current thread's errno slot, which
    // is always a valid readable pointer for the duration of the call.
    let location = unsafe { libc::__errno_location() };
    // SAFETY: `location` is the valid errno slot returned above and is read
    // before the call returns.
    Errno::new(unsafe { *location })
}
