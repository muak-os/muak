//! Control group vocabulary.
//!
//! The types always compile, because a policy must always be expressible. Only
//! the filesystem work in `v2` depends on the `cgroupv2` feature.
//!
//! Only the unified cgroup v2 hierarchy is supported. The kernel boots with
//! `cgroup_no_v1=all`, so a v1 hierarchy can never appear, and every facility
//! this crate relies on, `cgroup.kill` and `CLONE_INTO_CGROUP` above all, is a
//! v2 facility.

#[cfg(feature = "cgroupv2")]
pub mod v2;

// One byte below the kernel's limit for a single path component, so a name that
// passes validation always fits the buffer the kernel copies it into.
const NAME_LIMIT: usize = 256;

/// Limits a cgroup applies to the processes placed in it.
///
/// Every field is optional, and an absent field leaves the kernel default
/// alone, so a caller asks only for what it has to bound.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Limits {
    memory_max: Option<u64>,
    memory_high: Option<u64>,
    pids_max: Option<u64>,
    cpu_max: Option<(u64, u64)>,
}

impl Limits {
    /// Returns the set that requests no limit at all.
    #[must_use = "the limits are the value the caller asked for"]
    pub const fn new() -> Self {
        Self {
            memory_max: None,
            memory_high: None,
            pids_max: None,
            cpu_max: None,
        }
    }

    /// Returns these limits with `memory.max` set to `bytes`.
    ///
    /// `memory.max` is the hard limit: the kernel reclaims memory from a group
    /// that reaches it, and kills the group when reclaim cannot keep up.
    #[must_use = "the limits are the value the caller asked for"]
    pub const fn memory_max(mut self, bytes: u64) -> Self {
        self.memory_max = Some(bytes);
        self
    }

    /// Returns these limits with `memory.high` set to `bytes`.
    ///
    /// `memory.high` is the throttle: a group above it is reclaimed and slowed
    /// down, not killed.
    #[must_use = "the limits are the value the caller asked for"]
    pub const fn memory_high(mut self, bytes: u64) -> Self {
        self.memory_high = Some(bytes);
        self
    }

    /// Returns these limits with `pids.max` set to `count` processes.
    #[must_use = "the limits are the value the caller asked for"]
    pub const fn pids_max(mut self, count: u64) -> Self {
        self.pids_max = Some(count);
        self
    }

    /// Returns these limits with `cpu.max` set to `quota` microseconds of CPU
    /// time per `period` microseconds.
    #[must_use = "the limits are the value the caller asked for"]
    pub const fn cpu_max(mut self, quota: u64, period: u64) -> Self {
        self.cpu_max = Some((quota, period));
        self
    }

    /// Returns whether at least one limit is requested.
    #[must_use = "the answer decides whether the cgroup mechanism is needed"]
    pub(crate) const fn requested(&self) -> bool {
        self.memory_max.is_some()
            || self.memory_high.is_some()
            || self.pids_max.is_some()
            || self.cpu_max.is_some()
    }

    /// Returns whether `memory.max` or `pids.max` asks for a non zero value.
    ///
    /// Those two are the limits the kernel enforces on the cgroup itself, so
    /// requesting one is what makes the unified hierarchy mandatory.
    #[cfg(feature = "spawn")]
    #[must_use = "the answer decides whether the hierarchy is required"]
    pub(crate) const fn enforces(&self) -> bool {
        matches!(self.memory_max, Some(bytes) if bytes > 0)
            || matches!(self.pids_max, Some(count) if count > 0)
    }

    // Renders every requested limit to the file it is written to and the text
    // that goes into it, in one place, so the formatting is testable without a
    // filesystem.
    #[cfg(any(feature = "cgroupv2", test))]
    fn rendered(&self) -> [Option<(&'static str, String)>; 4] {
        [
            self.memory_max
                .map(|bytes| ("memory.max", bytes.to_string())),
            self.memory_high
                .map(|bytes| ("memory.high", bytes.to_string())),
            self.pids_max.map(|count| ("pids.max", count.to_string())),
            self.cpu_max
                .map(|(quota, period)| ("cpu.max", format!("{quota} {period}"))),
        ]
    }
}

/// Returns whether `name` names a single cgroup below a root.
///
/// A valid name is not empty, is neither `.` nor `..`, holds no `/`, and is
/// shorter than 256 bytes. A name is checked before anything touches the
/// filesystem, so an invalid one can never create or remove a directory.
#[must_use = "the answer decides whether the name may reach the filesystem"]
pub fn valid_name(name: &str) -> bool {
    if name.is_empty() || name == "." || name == ".." || name.len() >= NAME_LIMIT {
        return false;
    }
    !name.contains('/')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_requested_limit_renders_its_own_file() {
        // ARRANGE
        let limits = Limits::new()
            .memory_max(64 * 1024 * 1024)
            .memory_high(32 * 1024 * 1024)
            .pids_max(64)
            .cpu_max(50_000, 100_000);

        // ACT
        let rendered: Vec<(&str, String)> = limits.rendered().into_iter().flatten().collect();

        // ASSERT
        assert_eq!(
            rendered,
            [
                ("memory.max", "67108864".to_owned()),
                ("memory.high", "33554432".to_owned()),
                ("pids.max", "64".to_owned()),
                ("cpu.max", "50000 100000".to_owned()),
            ]
        );
    }

    #[test]
    fn a_cpu_limit_renders_two_numbers() {
        // ARRANGE
        let limits = Limits::new().cpu_max(2, 3);

        // ACT
        let rendered: Vec<(&str, String)> = limits.rendered().into_iter().flatten().collect();

        // ASSERT
        assert_eq!(rendered, [("cpu.max", "2 3".to_owned())]);
    }

    #[test]
    fn an_absent_limit_renders_nothing() {
        // ARRANGE
        let limits = Limits::new();

        // ACT
        let rendered = limits.rendered();

        // ASSERT
        assert!(rendered.iter().all(Option::is_none));
    }

    #[test]
    fn an_invalid_name_is_rejected() {
        // ARRANGE
        let at_the_limit = "a".repeat(NAME_LIMIT);
        let longer = "a".repeat(NAME_LIMIT + 1);
        let names = [
            "",
            ".",
            "..",
            "/",
            "a/b",
            "a/../b",
            at_the_limit.as_str(),
            longer.as_str(),
        ];

        // ACT
        let accepted = names.map(valid_name);

        // ASSERT
        assert!(
            accepted.iter().all(|valid| !valid),
            "{names:?} {accepted:?}"
        );
    }

    #[test]
    fn a_plain_name_is_accepted() {
        // ARRANGE
        let just_below = "a".repeat(NAME_LIMIT - 1);
        let names = ["workload", "workload-1", "a.b", "0", just_below.as_str()];

        // ACT
        let accepted = names.map(valid_name);

        // ASSERT
        assert!(
            accepted.iter().all(|valid| *valid),
            "{names:?} {accepted:?}"
        );
    }
}
