//! The `[sandbox]` table of a service file.

use anyhow::{Result, bail};
use sandbox::{cap, cgroup, credentials, limits, policy};
use serde::Deserialize;

/// Confinement a service is spawned under.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Sandbox {
    #[serde(default)]
    profile: Profile,
    user: Option<u32>,
    group: Option<u32>,
    capabilities: Option<Capabilities>,
    #[serde(default)]
    no_new_privs: bool,
    limits: Option<Limits>,
    cgroup: Option<Cgroup>,
}

/// Kind of confinement a service asks for.
#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Profile {
    #[default]
    Strict,
    None,
}

/// Capabilities a service keeps, named as a word or as a list of names.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
enum Capabilities {
    Word(CapWord),
    List(Vec<String>),
}

/// Whole set a service asks for in one word.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum CapWord {
    All,
    None,
}

/// Resource limits a service asks for, each absent value inherited.
#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Limits {
    nofile: Option<Amount>,
    nproc: Option<Amount>,
    memlock: Option<Amount>,
    address_space: Option<Amount>,
    core: Option<Amount>,
    file_size: Option<Amount>,
    stack: Option<Amount>,
}

// Control group limits a service asks for, each absent value left alone.
#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cgroup {
    pids_max: Option<u64>,
    memory_max: Option<u64>,
    memory_high: Option<u64>,
    cpu_quota: Option<u64>,
    cpu_period: Option<u64>,
}

/// One limit value, a number, or the word `unlimited`.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(untagged)]
enum Amount {
    Number(u64),
    Word(LimitWord),
}

/// Limit value that removes the limit.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum LimitWord {
    Unlimited,
}

impl Sandbox {
    /// Returns the policy this table describes.
    ///
    /// # Errors
    ///
    /// Returns an error when a table is malformed.
    pub(crate) fn policy(&self) -> Result<policy::Policy<'_>> {
        self.cgroup_limits()?;

        if matches!(self.profile, Profile::None) {
            return self.none_policy();
        }

        let mut policy = policy::Policy::default();
        if let Some(user) = self.user {
            let gid = self.group.unwrap_or(user);
            policy = policy.id(credentials::Id::new(user, gid).groups(&[]));
        }
        if let Some(caps) = self.caps()? {
            policy = policy.caps(caps);
        }
        if self.no_new_privs {
            policy = policy.no_new_privs();
        }
        if let Some(limits) = self.limits {
            policy = policy.limits(limits.policy());
        }

        Ok(policy)
    }

    /// Returns the cgroup limits this table requests.
    ///
    /// # Errors
    ///
    /// Returns an error naming the missing half when `cpu_quota` or `cpu_period` appears alone.
    pub(crate) fn cgroup_limits(&self) -> Result<cgroup::Limits> {
        match self.cgroup {
            Some(table) => table.limits(),
            None => Ok(cgroup::Limits::new()),
        }
    }

    fn none_policy(&self) -> Result<policy::Policy<'_>> {
        match self.ignored() {
            Some(key) => bail!("profile = \"none\" makes `{key}` do nothing; remove it"),
            None => Ok(policy::Policy::default()),
        }
    }

    const fn ignored(&self) -> Option<&'static str> {
        if self.user.is_some() {
            return Some("user");
        }
        if self.group.is_some() {
            return Some("group");
        }
        if self.capabilities.is_some() {
            return Some("capabilities");
        }
        if self.no_new_privs {
            return Some("no_new_privs");
        }
        if self.limits.is_some() {
            return Some("limits");
        }

        None
    }

    fn caps(&self) -> Result<Option<credentials::Caps>> {
        let Some(capabilities) = self.capabilities.clone() else {
            return Ok(None);
        };
        let caps = match capabilities {
            Capabilities::Word(CapWord::All) => credentials::Caps::all(),
            Capabilities::Word(CapWord::None) => credentials::Caps::none(),
            Capabilities::List(names) => named(&names)?,
        };

        Ok(Some(caps))
    }
}

impl Cgroup {
    fn limits(self) -> Result<cgroup::Limits> {
        let mut limits = cgroup::Limits::new();
        if let Some(count) = self.pids_max {
            limits = limits.pids_max(count);
        }
        if let Some(bytes) = self.memory_max {
            limits = limits.memory_max(bytes);
        }
        if let Some(bytes) = self.memory_high {
            limits = limits.memory_high(bytes);
        }
        match (self.cpu_quota, self.cpu_period) {
            (Some(quota), Some(period)) => limits = limits.cpu_max(quota, period),
            (None, None) => {}
            (Some(quota), None) => bail!("`cpu_quota = {quota}` also needs `cpu_period`"),
            (None, Some(period)) => bail!("`cpu_period = {period}` also needs `cpu_quota`"),
        }

        Ok(limits)
    }
}

impl Limits {
    fn policy(self) -> limits::Limits {
        limits::Limits::new()
            .nofile(limit(self.nofile))
            .nproc(limit(self.nproc))
            .memlock(limit(self.memlock))
            .address_space(limit(self.address_space))
            .core(limit(self.core))
            .file_size(limit(self.file_size))
            .stack(limit(self.stack))
    }
}

impl Amount {
    const fn limit(self) -> limits::Limit {
        match self {
            Self::Number(value) => limits::Limit::Value(value),
            Self::Word(LimitWord::Unlimited) => limits::Limit::Unlimited,
        }
    }
}

fn named(names: &[String]) -> Result<credentials::Caps> {
    let mut caps = credentials::Caps::none();
    for name in names {
        let Some(cap) = cap::Cap::from_name(name) else {
            bail!("unknown capability: {name}");
        };
        caps = caps.allow(cap);
    }

    Ok(caps)
}

fn limit(value: Option<Amount>) -> limits::Limit {
    value.map_or(limits::Limit::Inherit, Amount::limit)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PIDS: u8 = 0b0001;
    const MEMORY_MAX: u8 = 0b0010;
    const MEMORY_HIGH: u8 = 0b0100;
    const CPU: u8 = 0b1000;
    const ALL: u8 = PIDS | MEMORY_MAX | MEMORY_HIGH | CPU;

    // The table that asks for the settings `mask` selects, with the CPU setting written as the pair it is.
    fn cgroup_table(mask: u8) -> String {
        let mut table = String::from("[cgroup]\n");
        if mask & PIDS != 0 {
            table.push_str("pids_max = 3\n");
        }
        if mask & MEMORY_MAX != 0 {
            table.push_str("memory_max = 1024\n");
        }
        if mask & MEMORY_HIGH != 0 {
            table.push_str("memory_high = 512\n");
        }
        if mask & CPU != 0 {
            table.push_str("cpu_quota = 5\ncpu_period = 10\n");
        }
        table
    }

    // The limits `mask` must map to.
    fn expected_limits(mask: u8) -> cgroup::Limits {
        let mut limits = cgroup::Limits::new();
        if mask & PIDS != 0 {
            limits = limits.pids_max(3);
        }
        if mask & MEMORY_MAX != 0 {
            limits = limits.memory_max(1024);
        }
        if mask & MEMORY_HIGH != 0 {
            limits = limits.memory_high(512);
        }
        if mask & CPU != 0 {
            limits = limits.cpu_max(5, 10);
        }
        limits
    }

    #[test]
    fn every_combination_of_present_fields_maps_to_its_limit() {
        // ARRANGE
        let masks: Vec<u8> = (0..=ALL).collect();
        let tables: Vec<Sandbox> = masks
            .iter()
            .map(|mask| {
                let table = cgroup_table(*mask);
                toml::from_str::<Sandbox>(&table).expect("cgroup table")
            })
            .collect();

        // ACT
        let mapped: Vec<_> = tables.iter().map(Sandbox::cgroup_limits).collect();

        // ASSERT
        for (mask, limits) in masks.into_iter().zip(mapped) {
            let limits =
                limits.unwrap_or_else(|error| panic!("mask {mask:04b} was refused: {error:#}"));
            assert_eq!(limits, expected_limits(mask), "mask {mask:04b}");
        }
    }

    #[test]
    fn a_quota_without_a_period_is_refused() {
        // ARRANGE
        let table = "[cgroup]\ncpu_quota = 5\n";
        let sandbox = toml::from_str::<Sandbox>(table).expect("cgroup table");

        // ACT
        let error = sandbox.cgroup_limits().expect_err("half a CPU setting");

        // ASSERT
        let error = error.to_string();
        assert!(
            error.contains("cpu_quota") && error.contains("cpu_period"),
            "expected both keys in {error:?}"
        );
    }

    #[test]
    fn a_period_without_a_quota_is_refused() {
        // ARRANGE
        let table = "[cgroup]\ncpu_period = 10\n";
        let sandbox = toml::from_str::<Sandbox>(table).expect("cgroup table");

        // ACT
        let error = sandbox.policy().expect_err("half a CPU setting");

        // ASSERT
        let error = error.to_string();
        assert!(
            error.contains("cpu_period") && error.contains("cpu_quota"),
            "expected both keys in {error:?}"
        );
    }

    #[test]
    fn an_unknown_cgroup_key_is_refused() {
        // ARRANGE
        let table = "[cgroup]\npids_max = 8\nmemory_min = 1\n";

        // ACT
        let result = toml::from_str::<Sandbox>(table);

        // ASSERT
        assert!(result.is_err(), "an unknown key must not be ignored");
    }

    #[test]
    fn none_profile_keeps_its_cgroup_table() {
        // ARRANGE
        let table = "profile = \"none\"\n\n[cgroup]\npids_max = 8\n";
        let sandbox = toml::from_str::<Sandbox>(table).expect("cgroup table");

        // ACT
        let policy = sandbox.policy();

        // ASSERT
        assert!(
            policy.is_ok(),
            "a cgroup is not inert under profile none: {policy:?}"
        );
    }

    #[test]
    fn none_profile_rejects_an_inert_capability_list() {
        // ARRANGE
        let table = "profile = \"none\"\ncapabilities = [\"chown\"]\n";
        let sandbox = toml::from_str::<Sandbox>(table).unwrap();

        // ACT
        let error = sandbox.policy().unwrap_err().to_string();

        // ASSERT
        assert!(
            error.contains("`capabilities`") && error.contains("do nothing"),
            "expected the inert key in {error:?}"
        );
    }
}
