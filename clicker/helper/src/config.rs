//! Configuration written by the Noctalia plugin, parsed and validated.
//!
//! `Config::load` never hands out a value that the run loops would have to
//! second-guess: ranges are checked, unknown mouse buttons and bad key chords
//! are rejected here, and defaults live next to the fields they describe.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::ConfigError;
use crate::keys::{parse_chord, MouseButton};

pub const MAX_MACRO_STEPS: usize = 256;
pub const MAX_MACRO_REPEATS: u32 = 1_000_000;
pub const MAX_HOLD_MS: f64 = 2_000.0;
pub const MAX_STEP_DELAY_MS: f64 = 60_000.0;
pub const MAX_RUNTIME_S: f64 = 86_400.0;

pub const DEFAULT_CPS: f64 = 12.0;
pub const DEFAULT_HOLD_MS: f64 = 20.0;

pub const DEFAULT_CAST_BUTTON: MouseButton = MouseButton::Left;
pub const DEFAULT_PRESS_MS: f64 = 40.0;
pub const DEFAULT_SETTLE_MS: f64 = 1_200.0;
pub const DEFAULT_BITE_TIMEOUT_S: f64 = 30.0;
pub const DEFAULT_REEL_DELAY_MS: f64 = 33.0;
pub const DEFAULT_POST_REEL_MS: f64 = 600.0;
pub const DEFAULT_RECOVER_MS: f64 = 350.0;
pub const DEFAULT_SENSITIVITY: f64 = 0.70;
pub const DEFAULT_MIN_INTERVAL_MS: f64 = 1_200.0;
pub const MAX_FISHER_CASTS: u32 = 1_000_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Repeat,
    Macro,
    Fisher,
}

impl Mode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Repeat => "repeat",
            Self::Macro => "macro",
            Self::Fisher => "fisher",
        }
    }
}

/// What a single click of the repeat loop presses.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    Mouse {
        #[serde(default)]
        button: MouseButton,
    },
    Key {
        chord: String,
    },
}

impl Default for Action {
    fn default() -> Self {
        Self::Mouse {
            button: MouseButton::default(),
        }
    }
}

impl Action {
    /// Short label used by `status.json` and the dry-run report.
    pub fn label(&self) -> String {
        match self {
            Self::Mouse { button } => button.to_string(),
            Self::Key { chord } => format!("key:{chord}"),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScrollDirection {
    #[default]
    Down,
    Up,
}

impl ScrollDirection {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Down => "down",
            Self::Up => "up",
        }
    }

    /// Multiplication factor that turns a scroll step count into `REL_WHEEL`.
    pub const fn sign(self) -> i32 {
        match self {
            Self::Down => -1,
            Self::Up => 1,
        }
    }
}

impl fmt::Display for ScrollDirection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Step {
    Key {
        chord: String,
        #[serde(default)]
        hold_ms: f64,
        #[serde(default)]
        delay_ms: f64,
    },
    Mouse {
        #[serde(default)]
        button: MouseButton,
        #[serde(default)]
        hold_ms: f64,
        #[serde(default)]
        delay_ms: f64,
    },
    Delay {
        ms: f64,
    },
    Scroll {
        #[serde(default)]
        direction: ScrollDirection,
        #[serde(default = "default_scroll_steps")]
        steps: i32,
    },
}

impl Step {
    fn validate(&self, index: usize) -> Result<(), ConfigError> {
        match self {
            Self::Key {
                chord,
                hold_ms,
                delay_ms,
            } => {
                parse_chord(chord).map_err(|err| ConfigError::step(index, err.to_string()))?;
                validate_step_timing(index, "hold_ms", *hold_ms, MAX_HOLD_MS)?;
                validate_step_timing(index, "delay_ms", *delay_ms, MAX_STEP_DELAY_MS)?;
            }
            Self::Mouse {
                hold_ms, delay_ms, ..
            } => {
                validate_step_timing(index, "hold_ms", *hold_ms, MAX_HOLD_MS)?;
                validate_step_timing(index, "delay_ms", *delay_ms, MAX_STEP_DELAY_MS)?;
            }
            Self::Delay { ms } => {
                if !ms.is_finite() || !(0.0..=MAX_STEP_DELAY_MS).contains(ms) {
                    return Err(ConfigError::step(
                        index,
                        format!("delay ms must be between 0 and {MAX_STEP_DELAY_MS}"),
                    ));
                }
            }
            Self::Scroll { steps, .. } => {
                if !(1..=10).contains(steps) {
                    return Err(ConfigError::step(
                        index,
                        "scroll steps must be between 1 and 10",
                    ));
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MacroDef {
    #[serde(default)]
    pub name: String,
    pub steps: Vec<Step>,
    #[serde(default = "default_repeat", alias = "repeat")]
    pub repeats: u32,
    #[serde(default)]
    pub loop_delay_ms: f64,
    #[serde(default)]
    pub max_runtime_s: f64,
}

/// Timing and detection settings for the Terraria Fisher.
///
/// The Terraria Fisher casts, listens for the game's bite sound, reels in,
/// and repeats.
/// Every wait is explicit so the loop can be tuned without recompiling.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FisherConfig {
    /// Button that casts and reels. Terraria uses its Use / Attack button,
    /// which is the left mouse button by default.
    #[serde(default = "default_cast_button")]
    pub cast_button: MouseButton,

    /// How long the button stays down for one cast or reel.
    #[serde(default = "default_press_ms")]
    pub press_ms: f64,

    /// How long to ignore audio after a cast: the bobber is still in flight and
    /// its landing splash must not count as a bite.
    #[serde(default = "default_settle_ms")]
    pub settle_ms: f64,

    /// Give up waiting for a bite after this long and recast.
    #[serde(default = "default_bite_timeout_s")]
    pub bite_timeout_s: f64,

    /// Pause after hearing a bite before reeling in, so the game has finished
    /// registering it. About two frames at 60 FPS.
    #[serde(default = "default_reel_delay_ms")]
    pub reel_delay_ms: f64,

    /// Pause after reeling in before the next cast.
    #[serde(default = "default_post_reel_ms")]
    pub post_reel_ms: f64,

    /// Pause after an unsuccessful reel-in before recasting.
    #[serde(default = "default_recover_ms")]
    pub recover_ms: f64,

    /// Stop after this many casts. Zero means run until stopped.
    #[serde(default)]
    pub max_casts: u32,

    /// Correlation score (0..1) a detection must reach.
    #[serde(default = "default_sensitivity")]
    pub sensitivity: f64,

    /// Minimum gap between two detections, so one bite cannot reel twice.
    #[serde(default = "default_min_interval_ms")]
    pub min_interval_ms: f64,

    /// PulseAudio source to capture. Empty means the default sink's monitor.
    #[serde(default)]
    pub audio_device: String,

    /// Optional `.xnb` or raw f32 template. Empty means "find Terraria".
    #[serde(default)]
    pub template: String,

    /// Also correlate pitch-shifted copies of the template.
    #[serde(default = "default_pitch_variants")]
    pub pitch_variants: bool,
}

impl Default for FisherConfig {
    fn default() -> Self {
        Self {
            cast_button: DEFAULT_CAST_BUTTON,
            press_ms: DEFAULT_PRESS_MS,
            settle_ms: DEFAULT_SETTLE_MS,
            bite_timeout_s: DEFAULT_BITE_TIMEOUT_S,
            reel_delay_ms: DEFAULT_REEL_DELAY_MS,
            post_reel_ms: DEFAULT_POST_REEL_MS,
            recover_ms: DEFAULT_RECOVER_MS,
            max_casts: 0,
            sensitivity: DEFAULT_SENSITIVITY,
            min_interval_ms: DEFAULT_MIN_INTERVAL_MS,
            audio_device: String::new(),
            template: String::new(),
            pitch_variants: true,
        }
    }
}

impl FisherConfig {
    fn validate(&self) -> Result<(), ConfigError> {
        validate_range("fisher.press_ms", self.press_ms, 5.0, 500.0)?;
        validate_range("fisher.settle_ms", self.settle_ms, 0.0, 5_000.0)?;
        validate_range("fisher.bite_timeout_s", self.bite_timeout_s, 3.0, 300.0)?;
        validate_range("fisher.reel_delay_ms", self.reel_delay_ms, 0.0, 200.0)?;
        validate_range("fisher.post_reel_ms", self.post_reel_ms, 100.0, 5_000.0)?;
        validate_range("fisher.recover_ms", self.recover_ms, 0.0, 5_000.0)?;
        validate_range("fisher.sensitivity", self.sensitivity, 0.1, 0.99)?;
        validate_range(
            "fisher.min_interval_ms",
            self.min_interval_ms,
            100.0,
            10_000.0,
        )?;
        if self.max_casts > MAX_FISHER_CASTS {
            return Err(ConfigError::invalid(format!(
                "fisher.max_casts must be at most {MAX_FISHER_CASTS}"
            )));
        }
        Ok(())
    }
}

fn validate_range(name: &str, value: f64, minimum: f64, maximum: f64) -> Result<(), ConfigError> {
    if !value.is_finite() || !(minimum..=maximum).contains(&value) {
        return Err(ConfigError::invalid(format!(
            "{name} must be between {minimum} and {maximum}"
        )));
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub mode: Mode,
    #[serde(default = "default_cps")]
    pub cps: f64,
    #[serde(default = "default_hold")]
    pub hold_ms: f64,
    #[serde(default)]
    pub jitter_pct: f64,
    #[serde(default)]
    pub max_runtime_s: f64,
    #[serde(default)]
    pub action: Action,
    #[serde(default, rename = "macro")]
    pub macro_def: Option<MacroDef>,
    #[serde(default)]
    pub fisher: FisherConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            mode: Mode::Repeat,
            cps: DEFAULT_CPS,
            hold_ms: DEFAULT_HOLD_MS,
            jitter_pct: 0.0,
            max_runtime_s: 0.0,
            action: Action::default(),
            macro_def: None,
            fisher: FisherConfig::default(),
        }
    }
}

impl Config {
    /// Reads, parses, and normalizes the helper configuration.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let raw = fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        let parsed: Self = serde_json::from_str(&raw).map_err(|source| ConfigError::Parse {
            path: path.to_path_buf(),
            source,
        })?;
        parsed.normalized()
    }

    /// Parses configuration from memory. Useful in tests and for `--dry-run`.
    pub fn from_json(raw: &str) -> Result<Self, ConfigError> {
        let parsed: Self = serde_json::from_str(raw).map_err(|source| ConfigError::Parse {
            path: PathBuf::from("<memory>"),
            source,
        })?;
        parsed.normalized()
    }

    /// Clamps soft limits and rejects values that cannot be used at all.
    pub fn normalized(mut self) -> Result<Self, ConfigError> {
        if !self.cps.is_finite() || self.cps <= 0.0 {
            return Err(ConfigError::invalid("cps must be a positive number"));
        }
        self.cps = self.cps.clamp(1.0, 1_000.0);

        if !self.hold_ms.is_finite() || self.hold_ms < 0.0 {
            return Err(ConfigError::invalid("hold_ms must be zero or positive"));
        }
        self.hold_ms = self.hold_ms.clamp(0.0, MAX_HOLD_MS);

        if !(0.0..=50.0).contains(&self.jitter_pct) {
            return Err(ConfigError::invalid("jitter_pct must be between 0 and 50"));
        }
        self.jitter_pct = self.jitter_pct.clamp(0.0, 50.0);

        if !(0.0..=MAX_RUNTIME_S).contains(&self.max_runtime_s) {
            return Err(ConfigError::invalid(format!(
                "max_runtime_s must be between 0 and {MAX_RUNTIME_S}"
            )));
        }
        self.max_runtime_s = self.max_runtime_s.clamp(0.0, MAX_RUNTIME_S);

        if let Action::Key { chord } = &self.action {
            parse_chord(chord)?;
        }

        if let Some(def) = &self.macro_def {
            validate_macro(def)?;
        }

        if self.mode == Mode::Fisher {
            self.fisher.validate()?;
        }

        if self.mode == Mode::Macro && self.macro_def.is_none() {
            return Err(ConfigError::invalid(
                "mode is macro but no macro is configured",
            ));
        }

        Ok(self)
    }

    /// Maximum runtime for one repeat run, or `None` when unlimited.
    pub fn repeat_limit(&self) -> Option<Duration> {
        (self.max_runtime_s > 0.0).then_some(Duration::from_secs_f64(self.max_runtime_s))
    }
}

fn validate_macro(def: &MacroDef) -> Result<(), ConfigError> {
    if def.steps.is_empty() {
        return Err(ConfigError::invalid("macro has no steps"));
    }
    if def.steps.len() > MAX_MACRO_STEPS {
        return Err(ConfigError::invalid(format!(
            "macro has more than {MAX_MACRO_STEPS} steps"
        )));
    }
    if def.repeats > MAX_MACRO_REPEATS {
        return Err(ConfigError::invalid("macro repeat is too large"));
    }
    if !def.loop_delay_ms.is_finite() || !(0.0..=MAX_STEP_DELAY_MS).contains(&def.loop_delay_ms) {
        return Err(ConfigError::invalid(format!(
            "macro loop_delay_ms must be between 0 and {MAX_STEP_DELAY_MS}"
        )));
    }
    if !def.max_runtime_s.is_finite() || !(0.0..=MAX_RUNTIME_S).contains(&def.max_runtime_s) {
        return Err(ConfigError::invalid(format!(
            "macro max_runtime_s must be between 0 and {MAX_RUNTIME_S}"
        )));
    }
    for (index, step) in def.steps.iter().enumerate() {
        step.validate(index + 1)?;
    }
    Ok(())
}

fn validate_step_timing(
    index: usize,
    name: &str,
    value: f64,
    maximum: f64,
) -> Result<(), ConfigError> {
    if !value.is_finite() || !(0.0..=maximum).contains(&value) {
        return Err(ConfigError::step(
            index,
            format!("{name} must be between 0 and {maximum}"),
        ));
    }
    Ok(())
}

fn default_cps() -> f64 {
    DEFAULT_CPS
}

fn default_cast_button() -> MouseButton {
    DEFAULT_CAST_BUTTON
}

fn default_press_ms() -> f64 {
    DEFAULT_PRESS_MS
}

fn default_settle_ms() -> f64 {
    DEFAULT_SETTLE_MS
}

fn default_bite_timeout_s() -> f64 {
    DEFAULT_BITE_TIMEOUT_S
}

fn default_reel_delay_ms() -> f64 {
    DEFAULT_REEL_DELAY_MS
}

fn default_post_reel_ms() -> f64 {
    DEFAULT_POST_REEL_MS
}

fn default_recover_ms() -> f64 {
    DEFAULT_RECOVER_MS
}

fn default_sensitivity() -> f64 {
    DEFAULT_SENSITIVITY
}

fn default_min_interval_ms() -> f64 {
    DEFAULT_MIN_INTERVAL_MS
}

fn default_pitch_variants() -> bool {
    true
}

fn default_hold() -> f64 {
    DEFAULT_HOLD_MS
}

fn default_repeat() -> u32 {
    1
}

fn default_scroll_steps() -> i32 {
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_plugin_settings() {
        let config = Config::default();
        assert_eq!(config.mode, Mode::Repeat);
        assert_eq!(config.cps, 12.0);
        assert_eq!(config.hold_ms, 20.0);
        assert_eq!(
            config.action,
            Action::Mouse {
                button: MouseButton::Left
            }
        );
    }

    #[test]
    fn fills_missing_fields_from_defaults() {
        let config = Config::from_json(r#"{"cps": 30}"#).unwrap();
        assert_eq!(config.cps, 30.0);
        assert_eq!(config.hold_ms, DEFAULT_HOLD_MS);
        assert_eq!(config.jitter_pct, 0.0);
    }

    #[test]
    fn fisher_reel_delay_defaults_and_bounds() {
        let config = Config::from_json(r#"{"mode": "fisher"}"#).unwrap();
        assert_eq!(config.fisher.reel_delay_ms, DEFAULT_REEL_DELAY_MS);
        assert!(Config::from_json(r#"{"mode": "fisher", "fisher": {"reel_delay_ms": 0}}"#).is_ok());
        assert!(
            Config::from_json(r#"{"mode": "fisher", "fisher": {"reel_delay_ms": 201}}"#).is_err()
        );
    }

    #[test]
    fn clamps_rates_above_the_supported_maximum() {
        let config = Config::from_json(r#"{"cps": 5000}"#).unwrap();
        assert_eq!(config.cps, 1_000.0);
    }

    #[test]
    fn rejects_impossible_timing() {
        assert!(Config::from_json(r#"{"cps": 0}"#).is_err());
        assert!(Config::from_json(r#"{"cps": -1}"#).is_err());
        assert!(Config::from_json(r#"{"hold_ms": -1}"#).is_err());
        assert!(Config::from_json(r#"{"jitter_pct": 90}"#).is_err());
        assert!(Config::from_json(r#"{"max_runtime_s": 90000}"#).is_err());
    }

    #[test]
    fn rejects_unknown_mouse_buttons_during_deserialization() {
        let error =
            Config::from_json(r#"{"action": {"type": "mouse", "button": "turbo"}}"#).unwrap_err();
        assert!(error.to_string().contains("unknown variant"), "{error}");
    }

    #[test]
    fn rejects_bad_chords_and_missing_macros() {
        assert!(Config::from_json(r#"{"action": {"type": "key", "chord": "a+b"}}"#).is_err());
        assert!(Config::from_json(r#"{"mode": "macro"}"#).is_err());
    }

    #[test]
    fn validates_macro_limits_and_steps() {
        assert!(Config::from_json(r#"{"mode":"macro","macro":{"steps":[]}}"#).is_err());

        let too_many =
            r#"{"mode":"macro","macro":{"steps":[{"type":"delay","ms":1}],"repeats":1000001}}"#;
        assert!(Config::from_json(too_many).is_err());

        let bad_step = r#"{"mode":"macro","macro":{"steps":[{"type":"scroll","steps":11}]}}"#;
        let error = Config::from_json(bad_step).unwrap_err();
        assert!(error.to_string().starts_with("step 1:"), "{error}");
    }

    #[test]
    fn accepts_a_prepared_macro() {
        let config = Config::from_json(
            r#"{
                "mode": "macro",
                "macro": {
                    "name": "mine",
                    "steps": [
                        {"type": "key", "chord": "ctrl+a", "hold_ms": 10, "delay_ms": 40},
                        {"type": "mouse", "button": "right", "hold_ms": 8},
                        {"type": "delay", "ms": 100},
                        {"type": "scroll", "direction": "up", "steps": 2}
                    ],
                    "repeats": 3,
                    "loop_delay_ms": 250
                }
            }"#,
        )
        .unwrap();
        assert_eq!(config.macro_def.as_ref().unwrap().steps.len(), 4);
    }

    #[test]
    fn repeat_limit_treats_zero_as_unlimited() {
        let unlimited = Config::from_json(r#"{"max_runtime_s": 0}"#).unwrap();
        assert_eq!(unlimited.repeat_limit(), None);

        let limited = Config::from_json(r#"{"max_runtime_s": 2.5}"#).unwrap();
        assert_eq!(limited.repeat_limit(), Some(Duration::from_millis(2_500)));
    }
}
