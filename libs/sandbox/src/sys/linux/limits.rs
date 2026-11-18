//! Resource limit syscalls.
//!
//! The soft and hard value are set to the same number, which keeps the child from raising the soft value above what the parent intended.

use rustix::process;

use crate::error::Errno;
use crate::limits::{Limit, Limits};

pub(crate) fn apply(limits: &Limits) -> Result<(), Errno> {
    set(process::Resource::Nofile, limits.nofile)?;
    set(process::Resource::Nproc, limits.nproc)?;
    set(process::Resource::Memlock, limits.memlock)?;
    set(process::Resource::As, limits.address_space)?;
    set(process::Resource::Core, limits.core)?;
    set(process::Resource::Fsize, limits.file_size)?;
    set(process::Resource::Stack, limits.stack)?;
    Ok(())
}

fn set(resource: process::Resource, limit: Limit) -> Result<(), Errno> {
    let (current, maximum) = match limit {
        Limit::Inherit => return Ok(()),
        Limit::Unlimited => (None, None),
        Limit::Value(value) => (Some(value), Some(value)),
    };
    process::setrlimit(resource, process::Rlimit { current, maximum }).map_err(Errno::from)
}
