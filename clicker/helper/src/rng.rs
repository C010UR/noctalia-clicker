//! A tiny xorshift PRNG for jitter.
//!
//! Jitter does not need cryptographic randomness, only a cheap, well-distributed
//! value in `[0, 1)`, so pulling in a dependency would be overkill.

use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug)]
pub struct Rng(u64);

/// Distinguishes helpers started in the same clock tick.
static SEED_COUNTER: AtomicU64 = AtomicU64::new(0);

impl Rng {
    pub fn new() -> Self {
        let nanos = crate::time::Instant::now().as_nanos();
        let sequence = SEED_COUNTER.fetch_add(1, Ordering::Relaxed);
        Self(seed(nanos ^ sequence.wrapping_mul(0x9e37_79b9_7f4a_7c15)))
    }
}

impl Default for Rng {
    fn default() -> Self {
        Self::new()
    }
}

impl Rng {
    /// Uniform value in `[0, 1)`.
    pub fn next_f64(&mut self) -> f64 {
        let mut state = self.0;
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        self.0 = state;
        (state >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Mixes entropy sources and guarantees a non-zero xorshift state.
fn seed(nanos: u64) -> u64 {
    let pid = u64::from(std::process::id());
    (nanos ^ (pid << 32)) | 0x9e37_79b9_7f4a_7c15
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn produces_values_inside_the_unit_interval() {
        let mut rng = Rng::new();
        for _ in 0..10_000 {
            let value = rng.next_f64();
            assert!((0.0..1.0).contains(&value), "{value}");
        }
    }

    #[test]
    fn instances_start_from_different_states() {
        let mut first = Rng::new();
        let mut second = Rng::new();
        assert_ne!(first.next_f64(), second.next_f64());
    }

    #[test]
    fn seed_is_never_zero() {
        assert_ne!(seed(0), 0);
    }
}
