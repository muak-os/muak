//! Scripted `Input`/`Effect` tests for the `DHCPv4` method.

mod common;

#[cfg(test)]
mod tests {
    use core::net::{IpAddr, Ipv4Addr, SocketAddr};
    use core::time::Duration;
    use std::io;
    use std::time::{Instant, SystemTime};

    use dhcp::{Phase, View};
    use ip_method::{
        AddressConfig, Command, Event, Input, Intent, Method as _, SocketKind, SocketSlot,
    };
    use netmodel::MethodState;

    use crate::common::{
        ADDRESS_A, ADDRESS_B, GATEWAY, SERVER, Trace, XID, ack, ack_payload, bind, closed, failure,
        frame, intents, kind, link, method, nak_payload, offer, opened, payload, pump, pump_with,
        sent, states, warned, xid_of,
    };

    const DISCOVER: u8 = 1;
    const REQUEST: u8 = 3;
    const CIADDR: usize = 12;

    fn at(now: Instant, secs: u64) -> Instant {
        now.checked_add(Duration::from_secs(secs)).unwrap()
    }

    fn configure(address: Ipv4Addr) -> Intent {
        Intent::Configure(AddressConfig {
            address: IpAddr::V4(address),
            prefix_len: 24,
        })
    }

    fn deconfigure(address: Ipv4Addr) -> Intent {
        Intent::Deconfigure(AddressConfig {
            address: IpAddr::V4(address),
            prefix_len: 24,
        })
    }

    fn route() -> Intent {
        Intent::EnsureDefaultRoute(IpAddr::V4(GATEWAY))
    }

    fn ciaddr(payload: &[u8]) -> Ipv4Addr {
        let Some(octets) = payload
            .get(CIADDR..CIADDR + 4)
            .and_then(|octets| octets.first_chunk::<4>())
        else {
            return Ipv4Addr::UNSPECIFIED;
        };

        Ipv4Addr::from(*octets)
    }

    #[test]
    fn dora_binds_and_applies_the_lease() {
        // ARRANGE
        let now = Instant::now();
        let mut method = method();

        // ACT
        let started = pump(&mut method, now, Input::Command(Command::Start));
        let discover = sent(&started, SocketSlot::Primary).unwrap();
        let offer = offer(xid_of(&discover), ADDRESS_A);
        let selected = frame(&mut method, now, SocketSlot::Primary, &offer, None);
        let request = sent(&selected, SocketSlot::Primary).unwrap();
        let ack = ack(xid_of(&request), ADDRESS_A);
        let bound = frame(&mut method, now, SocketSlot::Primary, &ack, None);
        let view = method.view();
        let wake = method.next_wake(now);

        // ASSERT
        assert_eq!(states(&started), [MethodState::Acquiring]);
        assert_eq!(kind(&payload(&discover)), DISCOVER);
        assert_eq!(kind(&payload(&request)), REQUEST);
        assert_eq!(intents(&bound), [configure(ADDRESS_A), route()]);
        assert_eq!(states(&bound), [MethodState::Bound]);
        assert!(
            matches!(
                bound.last(),
                Some(Trace::Event(Event::State(MethodState::Bound)))
            ),
            "the bound state follows the lease intents"
        );
        let Some(View { lease, phase }) = view else {
            panic!("the method holds a lease");
        };
        assert_eq!(lease.assigned_ip, ADDRESS_A);
        assert_eq!(lease.server_ip, SERVER);
        assert_eq!(lease.prefix_len, 24);
        assert_eq!(lease.gateway, Some(GATEWAY));
        assert_eq!(lease.dns_servers, [Ipv4Addr::new(8, 8, 8, 8)]);
        assert_eq!(lease.lease_time, Duration::from_secs(100));
        assert_eq!(lease.renewal_time, Duration::from_secs(50));
        assert_eq!(lease.rebind_time, Duration::from_secs(87));
        assert_eq!(lease.obtained_at, SystemTime::UNIX_EPOCH);
        assert_eq!(phase, Phase::Bound);
        assert_eq!(wake, Some(at(now, 50)));
    }

    #[test]
    fn dora_walks_the_socket_lifecycle_in_order() {
        // ARRANGE
        let now = Instant::now();
        let mut method = method();

        // ACT
        let started = pump(&mut method, now, Input::Command(Command::Start));
        let discover = sent(&started, SocketSlot::Primary).unwrap();
        let offer = offer(xid_of(&discover), ADDRESS_A);
        let selected = frame(&mut method, now, SocketSlot::Primary, &offer, None);
        let request = sent(&selected, SocketSlot::Primary).unwrap();
        let ack = ack(xid_of(&request), ADDRESS_A);
        let bound = frame(&mut method, now, SocketSlot::Primary, &ack, None);

        // ASSERT
        assert_eq!(
            opened(&started),
            [(SocketSlot::Primary, SocketKind::RawL2, None, None)]
        );
        assert!(matches!(started.first(), Some(Trace::Event(_))));
        assert!(matches!(started.get(1), Some(Trace::Opened(..))));
        assert!(matches!(started.get(2), Some(Trace::Entropy)));
        assert!(
            matches!(started.get(3), Some(Trace::Sent(SocketSlot::Primary, _))),
            "the DISCOVER follows the opened socket and its entropy"
        );
        assert!(matches!(selected.first(), Some(Trace::Entropy)));
        assert!(
            matches!(selected.get(1), Some(Trace::Sent(SocketSlot::Primary, _))),
            "the REQUEST follows the offer entropy"
        );
        assert_eq!(closed(&bound), [SocketSlot::Primary]);
        assert_eq!(states(&bound), [MethodState::Bound]);
        assert!(
            matches!(bound.first(), Some(Trace::Closed(SocketSlot::Primary))),
            "the primary socket closes before the lease events"
        );
        assert!(matches!(
            bound.last(),
            Some(Trace::Event(Event::State(MethodState::Bound)))
        ));
    }

    #[test]
    fn renew_uses_the_secondary_udp_socket() {
        // ARRANGE
        let now = Instant::now();
        let mut method = method();
        bind(&mut method, now, ADDRESS_A);
        let renew_at = at(now, 50);

        // ACT
        let renewed = pump(&mut method, renew_at, Input::Timer);
        let request = sent(&renewed, SocketSlot::Secondary).unwrap();
        let ack = ack_payload(XID, ADDRESS_A);
        let accepted = frame(
            &mut method,
            renew_at,
            SocketSlot::Secondary,
            &ack,
            Some(IpAddr::V4(SERVER)),
        );
        let wake = method.next_wake(renew_at);

        // ASSERT
        assert_eq!(
            opened(&renewed),
            [(
                SocketSlot::Secondary,
                SocketKind::Udp,
                Some(SocketAddr::from((ADDRESS_A, 68))),
                Some(SocketAddr::from((SERVER, 67)))
            )]
        );
        assert_eq!(kind(&request), REQUEST);
        assert_eq!(ciaddr(&request), ADDRESS_A, "renew carries ciaddr");
        assert_eq!(closed(&accepted), [SocketSlot::Secondary]);
        assert_eq!(intents(&accepted), [configure(ADDRESS_A), route()]);
        assert_eq!(states(&accepted), [MethodState::Bound]);
        assert_eq!(wake, Some(at(now, 100)), "the lease restarts its deadlines");
    }

    #[test]
    fn renew_timeout_rebinds_by_broadcast() {
        // ARRANGE
        let now = Instant::now();
        let mut method = method();
        bind(&mut method, now, ADDRESS_A);
        drop(pump(&mut method, at(now, 50), Input::Timer));

        // ACT
        let timeout = at(now, 60);
        let failed_renewal = pump(&mut method, timeout, Input::Timer);
        let wake = method.next_wake(timeout);
        let rebind_at = at(now, 87);
        let rebound = pump(&mut method, rebind_at, Input::Timer);
        let request = sent(&rebound, SocketSlot::Primary).unwrap();
        let ack = ack(XID, ADDRESS_B);
        let accepted = frame(&mut method, rebind_at, SocketSlot::Primary, &ack, None);

        // ASSERT
        assert!(warned(&failed_renewal), "the failed renewal is reported");
        assert_eq!(wake, Some(rebind_at));
        assert_eq!(
            opened(&rebound),
            [(SocketSlot::Primary, SocketKind::RawL2, None, None)]
        );
        assert_eq!(kind(&payload(&request)), REQUEST);
        assert_eq!(
            ciaddr(&payload(&request)),
            ADDRESS_A,
            "rebind carries the held address"
        );
        assert_eq!(
            intents(&accepted),
            [
                deconfigure(ADDRESS_A),
                Intent::RemoveDefaultRoute(IpAddr::V4(GATEWAY)),
                configure(ADDRESS_B),
                route()
            ],
            "the replaced lease is torn down before the new one"
        );
        assert_eq!(states(&accepted), [MethodState::Bound]);
    }

    #[test]
    fn expiry_deconfigures_and_rediscovers() {
        // ARRANGE
        let now = Instant::now();
        let mut method = method();
        bind(&mut method, now, ADDRESS_A);
        drop(pump(&mut method, at(now, 50), Input::Timer));
        drop(pump(&mut method, at(now, 60), Input::Timer));
        drop(pump(&mut method, at(now, 87), Input::Timer));

        // ACT
        let expired = pump(&mut method, at(now, 100), Input::Timer);
        let view = method.view();
        let wake = method.next_wake(at(now, 100));

        // ASSERT
        assert_eq!(
            intents(&expired),
            [
                deconfigure(ADDRESS_A),
                Intent::RemoveDefaultRoute(IpAddr::V4(GATEWAY))
            ]
        );
        assert_eq!(states(&expired), [MethodState::Acquiring]);
        assert!(view.is_none(), "the expired lease is dropped");
        assert_eq!(
            wake,
            Some(at(now, 110)),
            "discovery waits for the DISCOVER reply"
        );
    }

    #[test]
    fn nak_restarts_discovery_from_scratch() {
        // ARRANGE
        let now = Instant::now();
        let mut method = method();
        bind(&mut method, now, ADDRESS_A);
        let renew_at = at(now, 50);
        drop(pump(&mut method, renew_at, Input::Timer));

        // ACT
        let refusal = nak_payload(XID);
        let restarted = frame(
            &mut method,
            renew_at,
            SocketSlot::Secondary,
            &refusal,
            Some(IpAddr::V4(SERVER)),
        );
        let view = method.view();
        let discover = sent(&restarted, SocketSlot::Primary).unwrap();
        let offer = offer(xid_of(&discover), ADDRESS_B);
        let selected = frame(&mut method, renew_at, SocketSlot::Primary, &offer, None);
        let request = sent(&selected, SocketSlot::Primary).unwrap();
        let ack = ack(xid_of(&request), ADDRESS_B);
        let accepted = frame(&mut method, renew_at, SocketSlot::Primary, &ack, None);

        // ASSERT
        assert_eq!(
            intents(&restarted),
            [
                deconfigure(ADDRESS_A),
                Intent::RemoveDefaultRoute(IpAddr::V4(GATEWAY))
            ],
            "the refused lease is removed"
        );
        assert_eq!(states(&restarted), [MethodState::Acquiring]);
        assert!(view.is_none(), "the refused lease is dropped");
        assert_eq!(kind(&payload(&discover)), DISCOVER);
        assert_eq!(intents(&accepted), [configure(ADDRESS_B), route()]);
        assert_eq!(states(&accepted), [MethodState::Bound]);
    }

    #[test]
    fn failed_acquisition_retries_after_backoff() {
        // ARRANGE
        let now = Instant::now();
        let mut method = method();
        let started = pump(&mut method, now, Input::Command(Command::Start));
        let discover = sent(&started, SocketSlot::Primary).unwrap();

        // ACT
        let timeout = at(now, 10);
        let retried = pump(&mut method, timeout, Input::Timer);
        let wake = method.next_wake(timeout);
        let retry_at = at(now, 12);
        let resent = pump(&mut method, retry_at, Input::Timer);
        let retry = sent(&resent, SocketSlot::Primary).unwrap();

        // ASSERT
        assert!(warned(&retried));
        assert_eq!(wake, Some(retry_at), "the retry deadline is armed");
        assert_eq!(kind(&payload(&retry)), DISCOVER);
        assert_eq!(
            retry.len(),
            discover.len(),
            "retries reuse the fixed buffers"
        );
    }

    #[test]
    fn exhausted_acquisitions_degrade() {
        // ARRANGE
        let now = Instant::now();
        let mut method = method();
        drop(pump(&mut method, now, Input::Command(Command::Start)));

        // ACT
        drop(pump(&mut method, at(now, 10), Input::Timer));
        drop(pump(&mut method, at(now, 12), Input::Timer));
        drop(pump(&mut method, at(now, 22), Input::Timer));
        drop(pump(&mut method, at(now, 26), Input::Timer));
        let degraded = pump(&mut method, at(now, 36), Input::Timer);
        let wake = method.next_wake(at(now, 36));

        // ASSERT
        assert!(failure(&degraded).is_some());
        assert_eq!(closed(&degraded), [SocketSlot::Primary]);
        assert_eq!(states(&degraded), [], "the failure replaces a state event");
        assert_eq!(wake, None);
    }

    #[test]
    fn link_down_degrades_and_keeps_the_lease() {
        // ARRANGE
        let now = Instant::now();
        let mut method = method();
        bind(&mut method, now, ADDRESS_A);

        // ACT
        let events = pump(&mut method, now, Input::Command(Command::LinkDown));
        let view = method.view();
        let wake = method.next_wake(now);

        // ASSERT
        assert_eq!(states(&events), [MethodState::Degraded]);
        assert_eq!(intents(&events), []);
        assert!(matches!(
            view,
            Some(View {
                phase: Phase::Degraded,
                ..
            })
        ));
        assert_eq!(wake, None);
    }

    #[test]
    fn link_snapshot_drives_the_same_transitions() {
        // ARRANGE
        let now = Instant::now();
        let mut method = method();
        bind(&mut method, now, ADDRESS_A);
        let mut down = link();
        down.state = netmodel::LinkState::Down;

        // ACT
        let degraded = pump(&mut method, now, Input::Link(down));
        let mut up = link();
        up.state = netmodel::LinkState::Up;
        let recovered = pump(&mut method, now, Input::Link(up));

        // ASSERT
        assert_eq!(states(&degraded), [MethodState::Degraded]);
        assert_eq!(intents(&recovered), [configure(ADDRESS_A), route()]);
        assert_eq!(states(&recovered), [MethodState::Bound]);
    }

    #[test]
    fn link_cycle_preserves_the_lease_deadlines() {
        // ARRANGE
        let now = Instant::now();
        let mut method = method();
        bind(&mut method, now, ADDRESS_A);

        // ACT
        drop(pump(
            &mut method,
            at(now, 40),
            Input::Command(Command::LinkDown),
        ));
        let recovered = pump(&mut method, at(now, 45), Input::Command(Command::LinkUp));
        let wake = method.next_wake(at(now, 45));

        // ASSERT
        assert_eq!(intents(&recovered), [configure(ADDRESS_A), route()]);
        assert_eq!(states(&recovered), [MethodState::Bound]);
        assert_eq!(
            wake,
            Some(at(now, 50)),
            "deadlines stay anchored to the bind instant"
        );
    }

    #[test]
    fn lease_expiring_while_the_link_is_down_expires() {
        // ARRANGE
        let now = Instant::now();
        let mut method = method();
        bind(&mut method, now, ADDRESS_A);
        drop(pump(
            &mut method,
            at(now, 40),
            Input::Command(Command::LinkDown),
        ));

        // ACT
        let expired = pump(&mut method, at(now, 105), Input::Poll);
        let view = method.view();
        let wake = method.next_wake(at(now, 105));

        // ASSERT
        assert_eq!(
            intents(&expired),
            [
                deconfigure(ADDRESS_A),
                Intent::RemoveDefaultRoute(IpAddr::V4(GATEWAY))
            ]
        );
        assert!(view.is_none(), "the expired lease is dropped");
        assert_eq!(wake, None);
    }

    #[test]
    fn socket_error_during_an_exchange_arms_a_retry() {
        // ARRANGE
        let now = Instant::now();
        let mut method = method();
        drop(pump(&mut method, now, Input::Command(Command::Start)));
        let error = io::Error::other("primary socket failed");

        // ACT
        let errored = pump(
            &mut method,
            now,
            Input::SocketError {
                slot: SocketSlot::Primary,
                error,
            },
        );
        let wake = method.next_wake(now);

        // ASSERT
        assert!(warned(&errored));
        assert!(failure(&errored).is_none(), "the error is retryable");
        assert_eq!(wake, Some(at(now, 2)), "the acquisition retry is armed");
    }

    #[test]
    fn socket_error_while_bound_only_warns() {
        // ARRANGE
        let now = Instant::now();
        let mut method = method();
        bind(&mut method, now, ADDRESS_A);
        let error = io::Error::other("primary socket failed");

        // ACT
        let errored = pump(
            &mut method,
            now,
            Input::SocketError {
                slot: SocketSlot::Primary,
                error,
            },
        );
        let view = method.view();
        let wake = method.next_wake(now);

        // ASSERT
        assert!(warned(&errored));
        assert_eq!(states(&errored), []);
        assert!(matches!(
            view,
            Some(View {
                phase: Phase::Bound,
                ..
            })
        ));
        assert_eq!(wake, Some(at(now, 50)));
    }

    #[test]
    fn stop_removes_the_lease_and_reports_stopped() {
        // ARRANGE
        let now = Instant::now();
        let mut method = method();
        bind(&mut method, now, ADDRESS_A);

        // ACT
        let events = pump(&mut method, now, Input::Command(Command::Stop));
        let view = method.view();
        let wake = method.next_wake(now);

        // ASSERT
        assert_eq!(
            intents(&events),
            [
                deconfigure(ADDRESS_A),
                Intent::RemoveDefaultRoute(IpAddr::V4(GATEWAY))
            ]
        );
        assert_eq!(states(&events), [MethodState::Stopped]);
        assert!(view.is_none());
        assert_eq!(wake, None);
    }

    #[test]
    fn reconcile_reapplies_a_held_lease() {
        // ARRANGE
        let now = Instant::now();
        let mut method = method();
        bind(&mut method, now, ADDRESS_A);

        // ACT
        let events = pump(&mut method, now, Input::Command(Command::Reconcile));
        let wake = method.next_wake(now);

        // ASSERT
        assert_eq!(intents(&events), [configure(ADDRESS_A), route()]);
        assert_eq!(states(&events), [MethodState::Bound]);
        assert_eq!(wake, Some(at(now, 50)));
    }

    #[test]
    fn reconcile_without_a_lease_restarts_discovery() {
        // ARRANGE
        let now = Instant::now();
        let mut method = method();

        // ACT
        let events = pump(&mut method, now, Input::Command(Command::Reconcile));

        // ASSERT
        assert_eq!(states(&events), [MethodState::Acquiring]);
        assert_eq!(intents(&events), []);
    }

    #[test]
    fn reconcile_after_stop_keeps_the_method_stopped() {
        // ARRANGE
        let now = Instant::now();
        let mut method = method();
        bind(&mut method, now, ADDRESS_A);
        drop(pump(&mut method, now, Input::Command(Command::Stop)));

        // ACT
        let events = pump(&mut method, now, Input::Command(Command::Reconcile));
        let view = method.view();
        let wake = method.next_wake(now);

        // ASSERT
        assert!(events.is_empty(), "a stopped method ignores reconcile");
        assert!(view.is_none());
        assert_eq!(wake, None);
    }

    #[test]
    fn repeated_discovery_reuses_the_fixed_buffers() {
        // ARRANGE
        let now = Instant::now();
        let mut method = method();
        let first = pump(&mut method, now, Input::Command(Command::Start));
        let discover = sent(&first, SocketSlot::Primary).unwrap();
        drop(pump(&mut method, now, Input::Command(Command::Stop)));

        // ACT
        let second = pump_with(&mut method, now, Input::Command(Command::Start), XID + 1);
        let resent = sent(&second, SocketSlot::Primary).unwrap();

        // ASSERT
        assert_eq!(discover.len(), resent.len(), "the frame buffer is reused");
        let first_payload = payload(&discover);
        let second_payload = payload(&resent);
        assert_eq!(dhcp_proto::xid::extract(&first_payload), XID);
        assert_eq!(dhcp_proto::xid::extract(&second_payload), XID + 1);
        assert_eq!(
            first_payload.get(8..),
            second_payload.get(8..),
            "the message buffer starts from a clean header"
        );
    }

    #[test]
    fn idle_step_waits_without_a_view() {
        // ARRANGE
        let now = Instant::now();
        let mut method = method();

        // ACT
        let idle = method.step(now, SystemTime::UNIX_EPOCH, Input::Poll);

        // ASSERT
        assert!(matches!(idle, ip_method::Step::Wait));
        assert!(method.view().is_none());
        assert_eq!(method.next_wake(now), None);
    }
}
