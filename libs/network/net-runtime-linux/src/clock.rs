//! Tokio backed monotonic and wall clock.

use std::time::{Instant, SystemTime};

pub(crate) struct Clock;

impl Clock {
    pub(crate) const fn new() -> Self {
        Self
    }
}

impl net_runtime::Clock for super::Tokio {
    fn now(&self) -> Instant {
        self.clock.now()
    }

    fn wall_now(&self) -> SystemTime {
        self.clock.wall_now()
    }
}

impl net_runtime::Clock for Clock {
    fn now(&self) -> Instant {
        tokio::time::Instant::now().into_std()
    }

    fn wall_now(&self) -> SystemTime {
        SystemTime::now()
    }
}
