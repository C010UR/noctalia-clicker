//! Band-energy fingerprint for the Terraria bite sound.
//!
//! A raw sample-by-sample matched filter is fragile: the game's audio is
//! resampled by PipeWire before it reaches the monitor, which changes the phase
//! enough to destroy sample-level correlation even though the sound is clearly
//! the same. Instead the detector describes audio as short-time band energies.
//!
//! Each 10 ms frame becomes sixteen Goertzel band levels, log-scaled. Comparing
//! a window means removing each band's mean across the window (so only the
//! *shape* of the sound over time matters) and taking a cosine similarity with
//! the template. That is insensitive to phase, gain, and small sample-rate
//! differences. Terraria randomizes the splash pitch, so the detector compares
//! against a set of pitch-shifted copies of the same template.

use std::collections::VecDeque;
use std::time::Duration;

use crate::audio::SAMPLE_RATE;
use crate::time::Instant;

/// Band centre frequencies, roughly log-spaced from 200 Hz to 18 kHz.
const BANDS: [f32; 16] = [
    200.0, 300.0, 420.0, 580.0, 800.0, 1_100.0, 1_500.0, 2_100.0, 2_900.0, 4_000.0, 5_500.0,
    7_500.0, 10_000.0, 12_500.0, 15_000.0, 18_000.0,
];

/// Analysis frame. 10 ms keeps detection latency low.
const FRAME_SAMPLES: usize = (SAMPLE_RATE / 100) as usize;

/// How far back the matcher looks, in frames (±40 ms).
const MAX_LAG_FRAMES: usize = 4;

/// Frames kept for matching; comfortably more than the longest template.
const FRAME_HISTORY: usize = 64;

/// Raw samples kept for the level meter.
const LEVEL_SAMPLES: usize = (SAMPLE_RATE / 100) as usize;

/// Per-poll decay of the reported score, for a meter that falls smoothly.
const SCORE_DECAY: f32 = 0.85;

#[derive(Debug)]
struct Template {
    descriptor: Vec<f32>,
}

impl Template {
    /// Frames are fixed-width: sixteen bands each.
    fn frame_count(&self) -> usize {
        self.descriptor.len() / BANDS.len()
    }
}

/// Matched-filter state.
#[derive(Debug)]
pub struct Detector {
    templates: Vec<Template>,
    frames: VecDeque<[f32; 16]>,
    pending: Vec<f32>,
    level_samples: VecDeque<f32>,
    threshold: f32,
    min_interval: Duration,
    last_detection: Option<Instant>,
    seen: u64,
    level: f32,
    score: f32,
}

/// One poll's worth of feedback for `status.json`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Reading {
    /// Smoothed input level, 0..1 (log-scaled for the panel).
    pub level: f32,
    /// Best similarity seen recently, 0..1.
    pub score: f32,
    pub detected: bool,
}

impl Detector {
    /// Builds a detector from a template. `variants` adds pitch-shifted copies,
    /// which the game uses to randomize the splash.
    pub fn new(template: &[f32], threshold: f32, min_interval: Duration, variants: bool) -> Self {
        let out_len = (crate::audio::TEMPLATE_SECONDS * SAMPLE_RATE as f32) as usize;
        let shifts: &[f32] = if variants {
            &[-5.0, -4.0, -3.0, -2.0, -1.0, 0.0, 1.0, 2.0, 3.0, 4.0, 5.0]
        } else {
            &[0.0]
        };

        let mut templates = Vec::with_capacity(shifts.len());
        for &semitones in shifts {
            let shifted = pitch_variant(template, semitones, out_len);
            let built = build_template(&shifted);
            if built.frame_count() >= 3 {
                templates.push(built);
            }
        }

        Self {
            templates,
            frames: VecDeque::with_capacity(FRAME_HISTORY),
            pending: Vec::with_capacity(FRAME_SAMPLES),
            level_samples: VecDeque::with_capacity(LEVEL_SAMPLES),
            threshold: threshold.clamp(0.05, 0.99),
            min_interval,
            last_detection: None,
            seen: 0,
            level: 0.0,
            score: 0.0,
        }
    }

    /// Forgets the current audio and cooldown. Called after a cast so the
    /// bobber's own splash cannot trigger a reel.
    pub fn reset(&mut self, total: u64) {
        self.frames.clear();
        self.pending.clear();
        self.level_samples.clear();
        self.seen = total;
        self.last_detection = None;
        self.score = 0.0;
    }

    /// Consumes the newest samples and reports level, score, and a detection.
    pub fn poll(&mut self, samples: &[f32], total: u64, now: Instant) -> Reading {
        let new = total.saturating_sub(self.seen) as usize;
        let start = samples.len().saturating_sub(new);
        for &sample in &samples[start..] {
            self.pending.push(sample);
            self.level_samples.push_back(sample);
            if self.level_samples.len() > LEVEL_SAMPLES {
                self.level_samples.pop_front();
            }
            if self.pending.len() >= FRAME_SAMPLES {
                let frame = band_energies(&self.pending[..FRAME_SAMPLES]);
                self.pending.drain(..FRAME_SAMPLES);
                self.frames.push_back(frame);
                if self.frames.len() > FRAME_HISTORY {
                    self.frames.pop_front();
                }
            }
        }
        self.seen = total;

        self.update_level();
        self.score *= SCORE_DECAY;

        let best = self.best_match();
        self.score = self.score.max(best);

        let mut detected = false;
        if best >= self.threshold {
            let ready = self
                .last_detection
                .is_none_or(|last| now.saturating_duration_since(last) >= self.min_interval);
            if ready {
                self.last_detection = Some(now);
                detected = true;
            }
        }

        Reading {
            level: self.level,
            score: self.score,
            detected,
        }
    }

    fn best_match(&self) -> f32 {
        let mut best = 0.0_f32;
        for template in &self.templates {
            let length = template.frame_count();
            if self.frames.len() < length {
                continue;
            }
            for lag in 0..=MAX_LAG_FRAMES {
                let end = self.frames.len().saturating_sub(lag);
                if end < length {
                    break;
                }
                let window: Vec<[f32; 16]> = self
                    .frames
                    .iter()
                    .skip(end - length)
                    .take(length)
                    .copied()
                    .collect();
                let descriptor = describe(&window);
                best = best.max(cosine(&descriptor, &template.descriptor));
            }
        }
        best
    }

    fn update_level(&mut self) {
        if self.level_samples.is_empty() {
            self.level *= 0.8;
            return;
        }
        let rms = (self
            .level_samples
            .iter()
            .map(|value| value * value)
            .sum::<f32>()
            / self.level_samples.len() as f32)
            .sqrt();
        // Map roughly -60 dBFS..0 dBFS to 0..1 for the meter.
        let decibels = 20.0 * rms.max(1e-6).log10();
        let level = ((decibels + 60.0) / 60.0).clamp(0.0, 1.0);
        self.level = self.level * 0.6 + level * 0.4;
    }
}

fn build_template(samples: &[f32]) -> Template {
    let mut frames = Vec::new();
    for chunk in samples.chunks(FRAME_SAMPLES) {
        if chunk.len() < FRAME_SAMPLES {
            break;
        }
        frames.push(band_energies(chunk));
    }
    if frames.len() < 3 {
        return Template {
            descriptor: Vec::new(),
        };
    }
    Template {
        descriptor: describe(&frames),
    }
}

/// Sixteen log-scaled band magnitudes, Hann-windowed to limit leakage.
fn band_energies(frame: &[f32]) -> [f32; 16] {
    let mut out = [0.0_f32; 16];
    let length = frame.len();

    let windowed: Vec<f32> = frame
        .iter()
        .enumerate()
        .map(|(index, sample)| {
            let window = 0.5
                - 0.5 * (std::f32::consts::TAU * index as f32 / (length.max(2) - 1) as f32).cos();
            sample * window
        })
        .collect();

    for (band, frequency) in BANDS.iter().enumerate() {
        let coefficient = 2.0 * (std::f32::consts::TAU * frequency / SAMPLE_RATE as f32).cos();
        let mut first = 0.0_f32;
        let mut second = 0.0_f32;
        for sample in &windowed {
            let value = sample + coefficient * first - second;
            second = first;
            first = value;
        }
        let power = first * first + second * second - coefficient * first * second;
        out[band] = power.max(0.0).sqrt().ln_1p();
    }
    out
}

/// Turns a window of band frames into a comparable vector: remove each band's
/// mean over time, then normalize, so only the temporal shape matters.
fn describe(frames: &[[f32; 16]]) -> Vec<f32> {
    let count = frames.len();
    if count == 0 {
        return Vec::new();
    }
    let mut means = [0.0_f32; 16];
    for frame in frames {
        for (band, value) in frame.iter().enumerate() {
            means[band] += value;
        }
    }
    for mean in &mut means {
        *mean /= count as f32;
    }

    let mut out = Vec::with_capacity(count * 16);
    for frame in frames {
        for (band, value) in frame.iter().enumerate() {
            out.push(value - means[band]);
        }
    }
    normalize(&mut out);
    out
}

fn normalize(values: &mut [f32]) {
    let energy = values.iter().map(|value| value * value).sum::<f32>().sqrt();
    if energy <= 1e-12 {
        values.fill(0.0);
        return;
    }
    for value in values.iter_mut() {
        *value /= energy;
    }
}

fn cosine(left: &[f32], right: &[f32]) -> f32 {
    left.iter()
        .zip(right)
        .map(|(left, right)| left * right)
        .sum::<f32>()
        .clamp(-1.0, 1.0)
}

fn pitch_variant(source: &[f32], semitones: f32, out_len: usize) -> Vec<f32> {
    // Terraria changes pitch by changing playback rate, so the sample gets
    // shorter as it rises. Reading the source faster reproduces that exactly.
    let ratio = 2.0_f32.powf(semitones / 12.0);
    let mut out = Vec::with_capacity(out_len);
    for index in 0..out_len {
        let position = index as f32 * ratio;
        let base = position.floor() as usize;
        let fraction = position - base as f32;
        let first = source.get(base).copied().unwrap_or(0.0);
        let second = source.get(base + 1).copied().unwrap_or(first);
        out.push(first + (second - first) * fraction);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::SAMPLE_RATE;

    fn splash() -> Vec<f32> {
        let length = (SAMPLE_RATE as f32 * 0.12) as usize;
        (0..length)
            .map(|index| {
                let time = index as f32 / SAMPLE_RATE as f32;
                let envelope = (-time * 35.0).exp();
                envelope
                    * ((time * 1_900.0 * std::f32::consts::TAU).sin()
                        + 0.6 * (time * 4_300.0 * std::f32::consts::TAU).sin())
            })
            .collect()
    }

    fn noise(count: usize, amplitude: f32, seed: u32) -> Vec<f32> {
        let mut state = seed.max(1);
        (0..count)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                (state as f32 / u32::MAX as f32 - 0.5) * 2.0 * amplitude
            })
            .collect()
    }

    #[test]
    fn correlates_a_buried_splash() {
        let template = splash();
        let mut detector = Detector::new(&template, 0.5, Duration::from_millis(500), false);

        let mut audio = noise(SAMPLE_RATE as usize / 2, 0.02, 7);
        audio.extend_from_slice(&template);
        let total = audio.len() as u64;
        let reading = detector.poll(&audio, total, Instant::now());
        assert!(reading.detected, "score {}", reading.score);
        assert!(reading.score > 0.6, "score {}", reading.score);
    }

    #[test]
    fn ignores_unrelated_audio() {
        let template = splash();
        let mut detector = Detector::new(&template, 0.55, Duration::from_millis(500), false);
        let audio = noise(SAMPLE_RATE as usize / 2, 0.4, 3);
        let total = audio.len() as u64;
        let reading = detector.poll(&audio, total, Instant::now());
        assert!(!reading.detected, "score {}", reading.score);
    }

    #[test]
    fn refractory_period_prevents_double_reeling() {
        let template = splash();
        let mut detector = Detector::new(&template, 0.4, Duration::from_secs(2), false);
        let mut audio = noise(SAMPLE_RATE as usize / 2, 0.01, 11);
        audio.extend_from_slice(&template);
        audio.extend_from_slice(&template);
        let total = audio.len() as u64;

        assert!(detector.poll(&audio, total, Instant::now()).detected);
        assert!(!detector.poll(&audio, total, Instant::now()).detected);
    }

    #[test]
    fn reset_skips_already_heard_audio() {
        let template = splash();
        let mut detector = Detector::new(&template, 0.4, Duration::from_millis(10), false);
        let mut audio = noise(SAMPLE_RATE as usize / 2, 0.01, 5);
        audio.extend_from_slice(&template);
        let total = audio.len() as u64;
        assert!(detector.poll(&audio, total, Instant::now()).detected);

        detector.reset(total);
        let reading = detector.poll(&audio, total, Instant::now());
        assert!(!reading.detected);
        assert_eq!(detector.frames.len(), 0);
    }

    #[test]
    fn tolerates_pitch_and_gain_changes() {
        let template = splash();
        let mut detector = Detector::new(&template, 0.45, Duration::from_millis(10), true);
        let shifted = pitch_variant(&template, 4.0, template.len());
        let mut audio = noise(SAMPLE_RATE as usize / 4, 0.02, 19);
        audio.extend(shifted.iter().map(|value| value * 0.25));
        let total = audio.len() as u64;
        let reading = detector.poll(&audio, total, Instant::now());
        assert!(reading.detected, "score {}", reading.score);
    }

    #[test]
    fn level_tracks_loudness() {
        let template = splash();
        let mut detector = Detector::new(&template, 0.4, Duration::from_millis(10), false);
        let quiet = noise(SAMPLE_RATE as usize / 10, 0.001, 23);
        let quiet_level = detector
            .poll(&quiet, quiet.len() as u64, Instant::now())
            .level;
        let loud = noise(SAMPLE_RATE as usize / 10, 0.5, 29);
        let loud_level = detector
            .poll(
                &loud,
                quiet.len() as u64 + loud.len() as u64,
                Instant::now(),
            )
            .level;
        assert!(loud_level > quiet_level, "{quiet_level} {loud_level}");
    }
}
