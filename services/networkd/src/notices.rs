//! Per-interface deduplication of the notices the driver reports.

extern crate alloc;

use alloc::collections::BTreeMap;

use netmodel::Name;

/// Notices already logged for each interface.
#[derive(Debug, Default)]
pub(crate) struct Logged {
    interfaces: BTreeMap<Name, Vec<String>>,
}

impl Logged {
    /// Calls `emit` for every notice of `name` that was not logged yet.
    ///
    /// The recorded set is replaced with `notices`, because a snapshot keeps
    /// its notices until the next configure.
    pub(crate) fn unseen(&mut self, name: &Name, notices: &[String], mut emit: impl FnMut(&str)) {
        let logged = self.interfaces.entry(name.clone()).or_default();
        let fresh = notices.iter().filter(|notice| !logged.contains(notice));
        for notice in fresh {
            emit(notice);
        }

        logged.clear();
        logged.extend_from_slice(notices);
    }
}

#[cfg(test)]
mod tests {
    use netmodel::Name;

    use super::Logged;

    fn name(value: &str) -> Name {
        Name::new(value).expect("valid interface name")
    }

    fn notices(values: [&str; 2]) -> Vec<String> {
        values.map(String::from).to_vec()
    }

    #[test]
    fn unseen_emits_every_notice_on_the_first_visit() {
        // ARRANGE
        let mut logged = Logged::default();
        let current = notices(["DHCP renew failed", "SLAAC has no router"]);
        let mut emitted = Vec::new();

        // ACT
        logged.unseen(&name("eth0"), &current, |notice| {
            emitted.push(notice.to_owned());
        });

        // ASSERT
        assert_eq!(emitted, current);
    }

    #[test]
    fn unseen_skips_notices_already_logged() {
        // ARRANGE
        let mut logged = Logged::default();
        let current = notices(["DHCP renew failed", "SLAAC has no router"]);
        logged.unseen(&name("eth0"), &current, |_| {});
        let mut emitted = Vec::new();

        // ACT
        logged.unseen(&name("eth0"), &current, |notice| {
            emitted.push(notice.to_owned());
        });

        // ASSERT
        assert_eq!(emitted, Vec::<String>::new());
    }

    #[test]
    fn unseen_emits_only_the_new_notice() {
        // ARRANGE
        let mut logged = Logged::default();
        logged.unseen(
            &name("eth0"),
            &notices(["DHCP renew failed", "SLAAC has no router"]),
            |_| {},
        );
        let current = notices(["DHCP renew failed", "Link is degraded"]);
        let mut emitted = Vec::new();

        // ACT
        logged.unseen(&name("eth0"), &current, |notice| {
            emitted.push(notice.to_owned());
        });

        // ASSERT
        assert_eq!(emitted, ["Link is degraded"]);
    }

    #[test]
    fn unseen_keeps_interfaces_apart() {
        // ARRANGE
        let mut logged = Logged::default();
        let current = notices(["DHCP renew failed", "SLAAC has no router"]);
        logged.unseen(&name("eth0"), &current, |_| {});
        let mut emitted = Vec::new();

        // ACT
        logged.unseen(&name("eth1"), &current, |notice| {
            emitted.push(notice.to_owned());
        });

        // ASSERT
        assert_eq!(emitted, current);
    }

    #[test]
    fn unseen_emits_again_after_the_notices_are_cleared() {
        // ARRANGE
        let mut logged = Logged::default();
        let current = notices(["DHCP renew failed", "SLAAC has no router"]);
        logged.unseen(&name("eth0"), &current, |_| {});
        logged.unseen(&name("eth0"), &[], |_| {});
        let mut emitted = Vec::new();

        // ACT
        logged.unseen(&name("eth0"), &current, |notice| {
            emitted.push(notice.to_owned());
        });

        // ASSERT
        assert_eq!(emitted, current);
    }
}
