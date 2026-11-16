//! The `[sandbox]` table of a service file.
//!
//! Every service carries one. The table is required on purpose: a service file
//! without it is refused at load time, so a new service cannot land without a
//! confinement policy just because nobody remembered to write one.
//! `profile = "none"` is the explicit opt out, and it says so in the file
//! instead of relying on a missing section.
//!
//! This module is the configuration side only. It builds a policy and issues
//! no syscall: the caller resolves the policy against the probed capabilities,
//! and the spawner hands the resolved policy to the sandbox crate.

use anyhow::{Result, bail};
use sandbox::{cap, credentials, limits, policy};
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

/// One limit value, a number or the word `unlimited`.
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
    /// An absent capability table keeps every capability, which is what the
    /// child inherits when no step asks for a change.
    ///
    /// # Errors
    ///
    /// Returns an error naming the key a `profile = "none"` table also sets,
    /// because nothing would apply it, and an error naming the capability when
    /// a name in the list is not a Linux capability.
    pub(crate) fn policy(&self) -> Result<policy::Policy<'_>> {
        if matches!(self.profile, Profile::None) {
            return self.none_policy();
        }

        let mut policy = policy::Policy::default();
        if let Some(user) = self.user {
            let gid = self.group.unwrap_or(user);
            // An empty group list clears the inherited supplementary groups,
            // which nothing can undo once the identity is dropped.
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

    // The policy a `profile = "none"` table describes, refusing keys it would
    // ignore so an operator is told the setting does nothing.
    fn none_policy(&self) -> Result<policy::Policy<'_>> {
        match self.ignored() {
            Some(key) => bail!("profile = \"none\" makes `{key}` do nothing; remove it"),
            None => Ok(policy::Policy::default()),
        }
    }

    // The first key a `profile = "none"` table makes inert.
    fn ignored(&self) -> Option<&'static str> {
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
