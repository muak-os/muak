//! Credential, capability and no-new-privs syscalls.

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
    // SAFETY: the slice owns `groups.len()` readable `gid_t` values and stays borrowed for the call, and an empty slice is passed with a zero count, which clears every group without reading the pointer.
    let result = unsafe { libc::syscall(libc::SYS_setgroups, groups.len(), groups.as_ptr()) };

    check_i64(result)
}

pub(crate) fn gid(gid: u32) -> Result<(), Errno> {
    // SAFETY: the raw call reads only scalar arguments and never dereferences a pointer.
    let result = unsafe { libc::syscall(libc::SYS_setresgid, gid, gid, gid) };

    check_i64(result)
}

pub(crate) fn uid(uid: u32) -> Result<(), Errno> {
    // SAFETY: the raw call reads only scalar arguments and never dereferences a pointer.
    let result = unsafe { libc::syscall(libc::SYS_setresuid, uid, uid, uid) };

    check_i64(result)
}

pub(crate) fn no_new_privs() -> Result<(), Errno> {
    // SAFETY: the call reads only scalar arguments and no pointer is involved.
    let result = unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };

    check(result)
}

#[cfg(feature = "caps")]
pub(crate) fn bounding(kept: Caps) -> Result<(), Errno> {
    for cap in Cap::ALL {
        if kept.contains(cap) {
            continue;
        }
        let number = libc::c_ulong::from(cap.number());
        // SAFETY: dropping a bounding capability reads only the scalar capability number and never dereferences a pointer.
        let result = unsafe { libc::prctl(libc::PR_CAPBSET_DROP, number, 0, 0, 0) };
        check(result)?;
    }

    Ok(())
}

#[cfg(feature = "caps")]
pub(crate) fn effective(kept: Caps) -> Result<(), Errno> {
    let bits = kept.bits();
    let low = u32::try_from(bits & u64::from(u32::MAX)).unwrap_or(u32::MAX);
    let high = u32::try_from(bits >> 32_i32).unwrap_or(u32::MAX);
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
    // SAFETY: both pointers refer to initialized `repr(C)` values that outlive the call, and the header version asks for exactly the two `u32` words the data array carries.
    let result = unsafe { libc::syscall(libc::SYS_capset, &header, data.as_ptr()) };

    check_i64(result)
}

fn check(result: c_int) -> Result<(), Errno> {
    if result == 0 { Ok(()) } else { Err(last()) }
}

fn check_i64(result: libc::c_long) -> Result<(), Errno> {
    if result == 0 { Ok(()) } else { Err(last()) }
}

fn last() -> Errno {
    // SAFETY: `__errno_location` returns the current thread's errno slot, always a valid readable pointer.
    let location = unsafe { libc::__errno_location() };

    // SAFETY: `location` is the valid errno slot returned above.
    Errno::new(unsafe { *location })
}
