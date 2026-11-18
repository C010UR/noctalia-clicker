//! Monotonic time and wall-clock helpers.
//!
//! Sleeping uses `CLOCK_MONOTONIC` deadlines so a run never drifts when the
//! wall clock changes, and `clock_nanosleep` is deliberately called without
//! `SA_RESTART` equivalents: a signal interrupts the sleep so the run loop can
//! react immediately.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Monotonic timestamp in nanoseconds, backed by `CLOCK_MONOTONIC`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Instant(u64);

impl Instant {
    pub fn now() -> Self {
        let mut timespec = std::mem::MaybeUninit::<libc::timespec>::uninit();
        // SAFETY: `clock_gettime` writes a full `timespec` into the pointer and
        // cannot fail for `CLOCK_MONOTONIC`.
        unsafe {
            libc::clock_gettime(libc::CLOCK_MONOTONIC, timespec.as_mut_ptr());
        }
        // SAFETY: the call above initialized `timespec`.
        let timespec = unsafe { timespec.assume_init() };
        Self(timespec.tv_sec as u64 * 1_000_000_000 + timespec.tv_nsec as u64)
    }

    pub fn saturating_add(self, duration: Duration) -> Self {
        Self(self.0.saturating_add(as_nanos(duration)))
    }

    pub fn saturating_duration_since(self, earlier: Self) -> Duration {
        Duration::from_nanos(self.0.saturating_sub(earlier.0))
    }

    pub fn elapsed(self) -> Duration {
        Self::now().saturating_duration_since(self)
    }

    pub(crate) fn as_nanos(self) -> u64 {
        self.0
    }
}

/// Sleeps until a monotonic deadline. Signals interrupt the sleep early, which
/// is what lets SIGUSR2 stop a run without waiting for the current period.
pub fn sleep_until(deadline: Instant) {
    let timespec = libc::timespec {
        tv_sec: (deadline.0 / 1_000_000_000) as libc::time_t,
        tv_nsec: (deadline.0 % 1_000_000_000) as libc::c_long,
    };
    // SAFETY: `timespec` is a valid absolute `CLOCK_MONOTONIC` deadline.
    unsafe {
        libc::clock_nanosleep(
            libc::CLOCK_MONOTONIC,
            libc::TIMER_ABSTIME,
            &timespec,
            std::ptr::null_mut(),
        );
    }
}

pub fn sleep_for(duration: Duration) {
    if !duration.is_zero() {
        sleep_until(Instant::now().saturating_add(duration));
    }
}

/// Wall-clock milliseconds, for human-facing timestamps in `status.json`.
pub fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Converts a millisecond value to a duration, treating invalid input as zero.
pub fn millis(value: f64) -> Duration {
    if value.is_finite() && value > 0.0 {
        Duration::from_nanos((value * 1_000_000.0) as u64)
    } else {
        Duration::ZERO
    }
}

fn as_nanos(duration: Duration) -> u64 {
    duration.as_nanos().min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn millis_handles_invalid_and_negative_values() {
        assert_eq!(millis(-5.0), Duration::ZERO);
        assert_eq!(millis(f64::NAN), Duration::ZERO);
        assert_eq!(millis(16.5), Duration::from_micros(16_500));
    }

    #[test]
    fn instant_arithmetic_is_monotonic() {
        let start = Instant::now();
        sleep_for(Duration::from_millis(5));
        let elapsed = start.elapsed();
        assert!(elapsed >= Duration::from_millis(5), "{elapsed:?}");
        assert!(elapsed < Duration::from_secs(5), "{elapsed:?}");
    }

    #[test]
    fn saturating_duration_since_clamps_to_zero() {
        let start = Instant::now();
        assert_eq!(
            start
                .saturating_add(Duration::from_millis(1))
                .saturating_duration_since(start),
            Duration::from_millis(1)
        );
        assert_eq!(
            start.saturating_duration_since(start.saturating_add(Duration::from_millis(1))),
            Duration::ZERO
        );
    }
}
