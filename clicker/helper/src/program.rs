//! A validated configuration, prepared for the run loops.
//!
//! Preparation turns user-facing strings (key chords, mouse button names) into
//! kernel codes once, so the timing-critical loops only do timing and I/O.

use std::time::Duration;

use crate::config::{Action, Config, MacroDef, ScrollDirection, Step};
use crate::error::ConfigError;
use crate::keys::{parse_chord, KeyCode, MouseButton};
use crate::time::millis;

/// Whether a press targets a keyboard key or a mouse button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PressKind {
    Mouse,
    Key,
}

/// A press/release pair that is ready to send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreparedAction {
    Mouse(MouseButton),
    Chord(Vec<KeyCode>),
}

impl PreparedAction {
    fn from_config(action: &Action) -> Result<Self, ConfigError> {
        Ok(match action {
            Action::Mouse { button } => Self::Mouse(*button),
            Action::Key { chord } => Self::Chord(parse_chord(chord)?),
        })
    }

    pub fn kind(&self) -> PressKind {
        match self {
            Self::Mouse(_) => PressKind::Mouse,
            Self::Chord(_) => PressKind::Key,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreparedStep {
    Press {
        action: PreparedAction,
        hold: Duration,
        delay: Duration,
    },
    Delay(Duration),
    Scroll {
        direction: ScrollDirection,
        steps: i32,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedMacro {
    pub name: String,
    pub steps: Vec<PreparedStep>,
    pub repeats: u32,
    pub loop_delay: Duration,
    pub max_runtime: Option<Duration>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    pub repeat: PreparedAction,
    pub macro_def: Option<PreparedMacro>,
}

impl Program {
    /// Placeholder used before the first configuration is loaded.
    pub fn empty() -> Self {
        Self {
            repeat: PreparedAction::Mouse(MouseButton::default()),
            macro_def: None,
        }
    }

    pub fn new(config: &Config) -> Result<Self, ConfigError> {
        Ok(Self {
            repeat: PreparedAction::from_config(&config.action)?,
            macro_def: config
                .macro_def
                .as_ref()
                .map(PreparedMacro::new)
                .transpose()?,
        })
    }
}

impl PreparedMacro {
    fn new(def: &MacroDef) -> Result<Self, ConfigError> {
        let steps = def
            .steps
            .iter()
            .map(PreparedStep::new)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self {
            name: def.name.clone(),
            steps,
            repeats: def.repeats,
            loop_delay: millis(def.loop_delay_ms),
            max_runtime: (def.max_runtime_s > 0.0).then(|| millis(def.max_runtime_s * 1_000.0)),
        })
    }
}

impl PreparedStep {
    fn new(step: &Step) -> Result<Self, ConfigError> {
        Ok(match step {
            Step::Key {
                chord,
                hold_ms,
                delay_ms,
            } => Self::Press {
                action: PreparedAction::Chord(parse_chord(chord)?),
                hold: millis(*hold_ms),
                delay: millis(*delay_ms),
            },
            Step::Mouse {
                button,
                hold_ms,
                delay_ms,
            } => Self::Press {
                action: PreparedAction::Mouse(*button),
                hold: millis(*hold_ms),
                delay: millis(*delay_ms),
            },
            Step::Delay { ms } => Self::Delay(millis(*ms)),
            Step::Scroll { direction, steps } => Self::Scroll {
                direction: *direction,
                steps: *steps,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prepare(json: &str) -> Program {
        Program::new(&Config::from_json(json).unwrap()).unwrap()
    }

    #[test]
    fn prepares_mouse_and_key_presses() {
        let mouse = prepare(r#"{"action":{"type":"mouse","button":"right"}}"#);
        assert_eq!(mouse.repeat, PreparedAction::Mouse(MouseButton::Right));
        assert_eq!(mouse.repeat.kind(), PressKind::Mouse);

        let key = prepare(r#"{"action":{"type":"key","chord":"ctrl+a"}}"#);
        assert_eq!(
            key.repeat,
            PreparedAction::Chord(vec![KeyCode(29), KeyCode(30)])
        );
        assert_eq!(key.repeat.kind(), PressKind::Key);
    }

    #[test]
    fn prepares_macro_steps_once() {
        let program = prepare(
            r#"{
                "mode": "macro",
                "macro": {
                    "name": "demo",
                    "steps": [
                        {"type": "key", "chord": "shift+f1", "hold_ms": 10, "delay_ms": 40},
                        {"type": "mouse", "button": "middle", "hold_ms": 8},
                        {"type": "delay", "ms": 100},
                        {"type": "scroll", "direction": "up", "steps": 2}
                    ],
                    "repeats": 0,
                    "loop_delay_ms": 250,
                    "max_runtime_s": 600
                }
            }"#,
        );

        let def = program.macro_def.unwrap();
        assert_eq!(def.name, "demo");
        assert_eq!(def.steps.len(), 4);
        assert_eq!(def.loop_delay, Duration::from_millis(250));
        assert_eq!(def.max_runtime, Some(Duration::from_secs(600)));
        assert_eq!(def.repeats, 0);
        assert!(matches!(
            def.steps[0],
            PreparedStep::Press {
                hold,
                delay,
                ..
            } if hold == Duration::from_millis(10) && delay == Duration::from_millis(40)
        ));
        assert_eq!(
            def.steps[3],
            PreparedStep::Scroll {
                direction: ScrollDirection::Up,
                steps: 2
            }
        );
    }

    #[test]
    fn zero_macro_runtime_means_unlimited() {
        let program = prepare(
            r#"{"mode":"macro","macro":{"steps":[{"type":"delay","ms":1}],"max_runtime_s":0}}"#,
        );
        assert_eq!(program.macro_def.unwrap().max_runtime, None);
    }
}
