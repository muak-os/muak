//! Monotonic and wall clock backed by the Linux primitives.

use std::time::{Instant, SystemTime};

impl net_runtime::Clock for super::Tokio {
    fn now(&self) -> Instant {
        self.clock.now()
    }

    fn wall_now(&self) -> SystemTime {
        self.clock.wall_now()
    }
}
