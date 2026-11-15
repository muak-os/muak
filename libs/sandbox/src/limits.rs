//! Resource limits a workload receives before exec.
//!
//! The types always compile, because a policy must always be expressible. The
//! syscall side in [`crate::sys::linux`] is the only part that depends on the
//! `limits` feature. [`Limit::Inherit`] leaves the inherited value alone, so no
//! field needs an `Option` wrapper to say "do not touch this".

/// Value a single resource limit is set to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Limit {
    /// Leave the inherited soft and hard value untouched.
    #[default]
    Inherit,
    /// Remove the limit, which is the kernel's `RLIM_INFINITY`.
    Unlimited,
    /// Set both the soft and the hard limit to this value.
    Value(u64),
}

/// Resource limits a child receives before exec.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Limits {
    pub(crate) nofile: Limit,
    pub(crate) nproc: Limit,
    pub(crate) memlock: Limit,
    pub(crate) address_space: Limit,
    pub(crate) core: Limit,
    pub(crate) file_size: Limit,
    pub(crate) stack: Limit,
}

impl Limits {
    /// Returns the set that leaves every inherited limit untouched.
    #[must_use = "the limits are the value the caller asked for"]
    pub const fn new() -> Self {
        Self {
            nofile: Limit::Inherit,
            nproc: Limit::Inherit,
            memlock: Limit::Inherit,
            address_space: Limit::Inherit,
            core: Limit::Inherit,
            file_size: Limit::Inherit,
            stack: Limit::Inherit,
        }
    }

    /// Returns these limits with the open file limit set to `limit`.
    #[must_use = "the limits are the value the caller asked for"]
    pub const fn nofile(mut self, limit: Limit) -> Self {
        self.nofile = limit;
        self
    }

    /// Returns these limits with the process count limit set to `limit`.
    #[must_use = "the limits are the value the caller asked for"]
    pub const fn nproc(mut self, limit: Limit) -> Self {
        self.nproc = limit;
        self
    }

    /// Returns these limits with the locked memory limit set to `limit`.
    #[must_use = "the limits are the value the caller asked for"]
    pub const fn memlock(mut self, limit: Limit) -> Self {
        self.memlock = limit;
        self
    }

    /// Returns these limits with the address space limit set to `limit`.
    #[must_use = "the limits are the value the caller asked for"]
    pub const fn address_space(mut self, limit: Limit) -> Self {
        self.address_space = limit;
        self
    }

    /// Returns these limits with the core dump limit set to `limit`.
    #[must_use = "the limits are the value the caller asked for"]
    pub const fn core(mut self, limit: Limit) -> Self {
        self.core = limit;
        self
    }

    /// Returns these limits with the file size limit set to `limit`.
    #[must_use = "the limits are the value the caller asked for"]
    pub const fn file_size(mut self, limit: Limit) -> Self {
        self.file_size = limit;
        self
    }

    /// Returns these limits with the stack size limit set to `limit`.
    #[must_use = "the limits are the value the caller asked for"]
    pub const fn stack(mut self, limit: Limit) -> Self {
        self.stack = limit;
        self
    }

    /// Returns whether at least one field is not [`Limit::Inherit`].
    #[cfg(feature = "spawn")]
    #[must_use = "the answer decides whether the limits mechanism is needed"]
    pub(crate) const fn requested(&self) -> bool {
        !matches!(self.nofile, Limit::Inherit)
            || !matches!(self.nproc, Limit::Inherit)
            || !matches!(self.memlock, Limit::Inherit)
            || !matches!(self.address_space, Limit::Inherit)
            || !matches!(self.core, Limit::Inherit)
            || !matches!(self.file_size, Limit::Inherit)
            || !matches!(self.stack, Limit::Inherit)
    }
}
