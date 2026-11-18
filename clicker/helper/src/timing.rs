//! Repeat-mode timing.
//!
//! Games that read the mouse once per frame (XNA/FNA titles such as Terraria)
//! miss presses shorter than a frame, and a fixed short hold can phase-lock
//! against the frame clock and register nothing at all. Each press therefore
//! lasts at least half the interval, capped at [`MIN_HOLD`], so it spans a
//! 60 Hz frame while leaving a real release gap.

use std::time::Duration;

/// Shortest press emitted in repeat mode.
pub const MIN_HOLD: Duration = Duration::from_millis(20);

/// Longest press as a fraction of the interval, so a release always happens.
const HOLD_FRACTION: f64 = 0.8;

/// Smallest period, and the fallback hold cap at extreme rates.
const MIN_PERIOD: Duration = Duration::from_millis(1);

/// Extra space left between release and the next press when jitter is on.
const RELEASE_GAP: Duration = Duration::from_micros(500);

/// Timing derived from a configured rate and hold.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RepeatTiming {
    period_ns: f64,
    hold_ns: f64,
}

impl RepeatTiming {
    pub fn new(cps: f64, requested_hold_ms: f64) -> Self {
        let cps = cps.clamp(1.0, 1_000.0);
        let period_ns = 1_000_000_000.0 / cps;
        let max_hold_ns = (period_ns * HOLD_FRACTION).max(as_nanos(MIN_PERIOD));
        let floor_ns = as_nanos(MIN_HOLD).min(period_ns / 2.0);
        let requested_ns = requested_hold_ms.max(0.0) * 1_000_000.0;

        Self {
            period_ns,
            hold_ns: requested_ns.max(floor_ns).min(max_hold_ns),
        }
    }

    pub fn period(self) -> Duration {
        Duration::from_nanos(self.period_ns as u64)
    }

    pub fn hold(self) -> Duration {
        Duration::from_nanos(self.hold_ns as u64)
    }

    /// Period for the next press, after jitter, never shorter than the hold.
    pub fn jittered_period(self, factor: f64) -> Duration {
        Duration::from_nanos((self.period_ns * factor.max(0.0)) as u64)
            .max(self.hold() + RELEASE_GAP)
    }

    pub fn effective_cps(self) -> f64 {
        1_000_000_000.0 / self.period_ns
    }

    pub fn effective_hold_ms(self) -> f64 {
        self.hold_ns / 1_000_000.0
    }

    pub fn effective_period_ms(self) -> f64 {
        self.period_ns / 1_000_000.0
    }
}

fn as_nanos(duration: Duration) -> f64 {
    duration.as_nanos() as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hold_ms(cps: f64, hold: f64) -> f64 {
        (RepeatTiming::new(cps, hold).effective_hold_ms() * 1_000.0).round() / 1_000.0
    }

    #[test]
    fn short_holds_are_lifted_to_half_the_interval() {
        // 30 CPS is 33.33 ms per click, so a 5 ms hold becomes 16.67 ms.
        assert_eq!(hold_ms(30.0, 5.0), 16.667);
        assert_eq!(hold_ms(30.0, 9.0), 16.667);
    }

    #[test]
    fn holds_between_the_floor_and_cap_are_kept() {
        assert_eq!(hold_ms(30.0, 25.0), 25.0);
    }

    #[test]
    fn holds_never_consume_the_whole_interval() {
        // 80% of 33.33 ms is 26.67 ms.
        assert_eq!(hold_ms(30.0, 60.0), 26.667);
    }

    #[test]
    fn slow_rates_use_the_full_twenty_millisecond_floor() {
        assert_eq!(hold_ms(12.0, 5.0), 20.0);
        assert_eq!(hold_ms(1.0, 0.0), 20.0);
    }

    #[test]
    fn extreme_rates_still_release() {
        let timing = RepeatTiming::new(1_000.0, 20.0);
        assert!(timing.hold() < timing.period() || timing.period() == MIN_PERIOD);
        assert!(timing.hold() <= Duration::from_millis(1));
    }

    #[test]
    fn effective_rate_round_trips() {
        let timing = RepeatTiming::new(30.0, 5.0);
        assert!((timing.effective_cps() - 30.0).abs() < 1e-9);
        assert!((timing.effective_period_ms() - 33.333_333).abs() < 1e-3);
    }

    #[test]
    fn jitter_never_overlaps_press_and_release() {
        let timing = RepeatTiming::new(60.0, 25.0);
        let shortest = timing.jittered_period(0.1);
        assert!(shortest >= timing.hold());
        assert!(shortest > timing.hold());
    }
}
