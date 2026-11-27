//! Monotonic and wall clock over `std::time`.

use std::time::{Instant, SystemTime};

/// Clock backed by the standard library.
#[derive(Debug, Clone, Copy, Default)]
pub struct Clock;

impl Clock {
    /// Creates a clock.
    #[must_use = "the runtime bundle needs a clock"]
    pub const fn new() -> Self {
        Self
    }
}

impl net_runtime::Clock for Clock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn wall_now(&self) -> SystemTime {
        SystemTime::now()
    }
}
