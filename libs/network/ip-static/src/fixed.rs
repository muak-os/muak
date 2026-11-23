//! Family-neutral static address provisioning method.

use std::time::{Instant, SystemTime};

use ip_method::{Input, Method, Step};

use crate::{Config, Family, Static, View};

impl<F: Family> Static<F> {
    /// Creates a static method from the given configuration.
    #[must_use = "the method owns the provisioning state"]
    pub const fn new(config: Config<F::Address>) -> Self {
        Self { config }
    }

    /// Returns the configuration the method was created with.
    #[must_use = "callers need the configuration to report it"]
    pub const fn config(&self) -> &Config<F::Address> {
        &self.config
    }
}

impl<F: Family> Method for Static<F> {
    type View = View<F::Address>;

    fn step(&mut self, _now: Instant, _wall: SystemTime, _input: Input<'_>) -> Step<'_> {
        Step::Wait
    }

    fn next_wake(&self, _now: Instant) -> Option<Instant> {
        None
    }

    fn view(&self) -> Option<Self::View> {
        None
    }
}

#[cfg(test)]
mod tests {
    use core::net::{IpAddr, Ipv4Addr};
    use std::time::{Instant, SystemTime};

    use ip_method::{Input, Method as _, Step};

    use crate::{Config, Family, Static};

    const ADDRESS: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 2);
    const GATEWAY: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 1);
    const DNS: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 53);

    struct V4;

    impl Family for V4 {
        type Address = Ipv4Addr;

        fn ip(address: Ipv4Addr) -> IpAddr {
            IpAddr::V4(address)
        }
    }

    fn config() -> Config<Ipv4Addr> {
        Config {
            addresses: vec![(ADDRESS, 24)],
            gateway: Some(GATEWAY),
            dns: vec![DNS],
        }
    }

    #[test]
    fn new_stores_config() {
        // ARRANGE
        let config = config();

        // ACT
        let method = Static::<V4>::new(config);

        // ASSERT
        assert_eq!(method.config().addresses, vec![(ADDRESS, 24)]);
        assert_eq!(method.config().gateway, Some(GATEWAY));
        assert_eq!(method.config().dns, vec![DNS]);
    }

    #[test]
    fn stub_step_waits_and_has_no_wake_or_view() {
        // ARRANGE
        let mut method = Static::<V4>::new(config());
        let now = Instant::now();

        // ACT
        let waits = matches!(
            method.step(now, SystemTime::UNIX_EPOCH, Input::Poll),
            Step::Wait
        );

        // ASSERT
        assert!(waits);
        assert!(method.next_wake(now).is_none());
        assert!(method.view().is_none());
    }
}
