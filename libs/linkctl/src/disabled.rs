//! Placeholder address family for builds that disable it.

use alloc::string::String;
use core::convert::Infallible;
use core::marker::PhantomData;
use std::time::Instant;

use netdev::interface::Ethernet;

/// Address family that is never configured because its feature is disabled.
pub struct Family<C, V>(Infallible, PhantomData<(C, V)>);

/// Address method of an address family that is never configured.
pub struct Never<V>(Infallible, PhantomData<V>);

impl<C: 'static, V: Send + 'static> crate::family::Family for Family<C, V> {
    const ENABLED: bool = false;

    type Config = C;
    type View = V;
    type Method = Never<V>;

    fn configure(_: C, _: &Ethernet) -> impl Future<Output = Result<Never<V>, String>> + Send {
        core::future::ready(Err(String::from("address family disabled")))
    }
}

impl<V: Send> addrconf::Method for Never<V> {
    type View = V;

    async fn next_event(&mut self, _now: Instant) -> addrconf::Event {
        match self.0 {}
    }

    fn next_wake(&self, _now: Instant) -> Option<Instant> {
        match self.0 {}
    }

    fn on_command(&mut self, _command: addrconf::Command) {
        match self.0 {}
    }

    fn view(&self) -> Option<V> {
        match self.0 {}
    }
}
