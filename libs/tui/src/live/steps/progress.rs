//! Timed step progress.

use core::time::Duration;
use std::time::Instant;

use crate::progress::Unit;

const RATE_WINDOW: Duration = Duration::from_secs(1);

pub(super) struct Progress {
    pub(super) current: u64,
    pub(super) total: u64,
    pub(super) unit: Unit,
    pub(super) rate: Option<u64>,
    pub(super) eta: Option<Duration>,
    started: Instant,
}

impl Progress {
    pub(super) fn new(total: u64, unit: Unit) -> Self {
        Self {
            current: 0,
            total,
            unit,
            rate: None,
            eta: None,
            started: Instant::now(),
        }
    }

    pub(super) fn set(&mut self, current: u64) {
        self.current = current;
        self.sample(Instant::now());
    }

    fn sample(&mut self, now: Instant) {
        if self.unit != Unit::Bytes {
            return;
        }

        let elapsed = now.saturating_duration_since(self.started);
        if elapsed < RATE_WINDOW {
            return;
        }

        let millis = elapsed.as_millis();
        if millis == 0 {
            return;
        }

        let rate = u128::from(self.current)
            .checked_mul(1000)
            .and_then(|scaled| scaled.checked_div(millis))
            .and_then(|rate| u64::try_from(rate).ok())
            .unwrap_or(u64::MAX);
        self.rate = Some(rate);
        self.eta = remaining_eta(self.total, self.current, rate);
    }
}

fn remaining_eta(total: u64, current: u64, rate: u64) -> Option<Duration> {
    if rate == 0 {
        return None;
    }

    let remaining = total.checked_sub(current)?;
    let seconds = remaining.checked_div(rate)?;

    Some(Duration::from_secs(seconds))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aged(progress: &mut Progress) {
        progress.started = Instant::now()
            .checked_sub(Duration::from_secs(2))
            .expect("instant is old enough");
    }

    #[test]
    fn bytes_progress_reports_a_smoothed_rate_and_eta() {
        // ARRANGE
        let mut progress = Progress::new(200, Unit::Bytes);
        progress.current = 100;
        aged(&mut progress);

        // ACT
        progress.sample(Instant::now());

        // ASSERT
        assert!(progress.rate.is_some_and(|rate| rate > 0));
        assert!(progress.eta.is_some_and(|eta| eta > Duration::ZERO));
    }

    #[test]
    fn item_progress_never_reports_rate_or_eta() {
        // ARRANGE
        let mut progress = Progress::new(10, Unit::Items);
        progress.current = 5;
        aged(&mut progress);

        // ACT
        progress.sample(Instant::now());

        // ASSERT
        assert_eq!(progress.rate, None);
        assert_eq!(progress.eta, None);
    }

    #[test]
    fn rate_is_hidden_before_the_sample_window() {
        // ARRANGE
        let mut progress = Progress::new(200, Unit::Bytes);
        progress.current = 100;

        // ACT
        progress.sample(Instant::now());

        // ASSERT
        assert_eq!(progress.rate, None);
        assert_eq!(progress.eta, None);
    }
}
