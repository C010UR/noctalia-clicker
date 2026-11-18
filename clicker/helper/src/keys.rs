//! Linux input event codes, mouse buttons, and key-chord parsing.
//!
//! Codes come from `/usr/include/linux/input-event-codes.h`. Chords are written
//! the way users type them (`ctrl+shift+a`, `space`, `f24`, `0x2a`) and are
//! parsed once when configuration is loaded.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::ConfigError;

/// A Linux input event key code (`EV_KEY` code space).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KeyCode(pub u16);

impl KeyCode {
    pub const fn as_u16(self) -> u16 {
        self.0
    }
}

/// Mouse buttons supported by the helper, in kernel code order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MouseButton {
    #[default]
    Left,
    Right,
    Middle,
    Side,
    Extra,
    Forward,
    Back,
    Task,
}

impl MouseButton {
    pub const ALL: [Self; 8] = [
        Self::Left,
        Self::Right,
        Self::Middle,
        Self::Side,
        Self::Extra,
        Self::Forward,
        Self::Back,
        Self::Task,
    ];

    pub const fn code(self) -> KeyCode {
        KeyCode(match self {
            Self::Left => 272,
            Self::Right => 273,
            Self::Middle => 274,
            Self::Side => 275,
            Self::Extra => 276,
            Self::Forward => 277,
            Self::Back => 278,
            Self::Task => 279,
        })
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Middle => "middle",
            Self::Side => "side",
            Self::Extra => "extra",
            Self::Forward => "forward",
            Self::Back => "back",
            Self::Task => "task",
        }
    }

    /// Parses a button name case-insensitively, for command line use.
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.to_ascii_lowercase();
        Self::ALL.into_iter().find(|button| button.name() == value)
    }
}

impl fmt::Display for MouseButton {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// Every button and key the helper might hold, for `--release-all`.
pub const SAFETY_RELEASE: [KeyCode; 13] = [
    KeyCode(272), // BTN_LEFT
    KeyCode(273), // BTN_RIGHT
    KeyCode(274), // BTN_MIDDLE
    KeyCode(275), // BTN_SIDE
    KeyCode(276), // BTN_EXTRA
    KeyCode(277), // BTN_FORWARD
    KeyCode(278), // BTN_BACK
    KeyCode(279), // BTN_TASK
    KeyCode(29),  // KEY_LEFTCTRL
    KeyCode(42),  // KEY_LEFTSHIFT
    KeyCode(56),  // KEY_LEFTALT
    KeyCode(125), // KEY_LEFTMETA
    KeyCode(57),  // KEY_SPACE
];

fn modifier_code(token: &str) -> Option<KeyCode> {
    Some(KeyCode(match token {
        "ctrl" | "control" => 29,
        "shift" => 42,
        "alt" => 56,
        "super" | "meta" | "win" | "logo" => 125,
        _ => return None,
    }))
}

/// Parses one key name: a named key, a single character, decimal, or hex.
fn key_code(name: &str) -> Option<KeyCode> {
    let token = name.to_ascii_lowercase();

    if let Some(hex) = token.strip_prefix("0x") {
        return u16::from_str_radix(hex, 16)
            .ok()
            .filter(|&code| code > 0)
            .map(KeyCode);
    }

    if token.bytes().all(|byte| byte.is_ascii_digit()) {
        if let Ok(code) = token.parse::<u16>() {
            if code > 0 {
                return Some(KeyCode(code));
            }
        }
    }

    if token.len() == 1 {
        let byte = token.as_bytes()[0];
        if byte.is_ascii_lowercase() {
            return Some(KeyCode(30 + u16::from(byte - b'a')));
        }
        if byte.is_ascii_digit() {
            return Some(KeyCode(match byte {
                b'1' => 2,
                b'2' => 3,
                b'3' => 4,
                b'4' => 5,
                b'5' => 6,
                b'6' => 7,
                b'7' => 8,
                b'8' => 9,
                b'9' => 10,
                _ => 11, // KEY_0
            }));
        }
        return Some(KeyCode(match token.as_str() {
            "-" => 12,
            "=" => 13,
            "," => 51,
            "." => 52,
            "/" => 53,
            ";" => 39,
            "'" => 40,
            "\\" => 43,
            "`" => 41,
            "[" => 26,
            "]" => 27,
            _ => return None,
        }));
    }

    Some(KeyCode(match token.as_str() {
        "space" => 57,
        "enter" | "return" => 28,
        "tab" => 15,
        "esc" | "escape" => 1,
        "backspace" => 14,
        "delete" | "del" => 111,
        "insert" | "ins" => 110,
        "home" => 102,
        "end" => 107,
        "pageup" | "pgup" => 104,
        "pagedown" | "pgdn" => 109,
        "up" => 103,
        "down" => 108,
        "left" => 105,
        "right" => 106,
        "minus" => 12,
        "equal" => 13,
        "comma" => 51,
        "dot" | "period" => 52,
        "slash" => 53,
        "semicolon" => 39,
        "quote" | "apostrophe" => 40,
        "backslash" => 43,
        "backtick" | "grave" => 41,
        "leftbracket" | "leftbrace" => 26,
        "rightbracket" | "rightbrace" => 27,
        "capslock" => 58,
        "numlock" => 69,
        "scrolllock" => 70,
        "printscreen" | "sysrq" => 99,
        "pause" => 119,
        "f1" => 59,
        "f2" => 60,
        "f3" => 61,
        "f4" => 62,
        "f5" => 63,
        "f6" => 64,
        "f7" => 65,
        "f8" => 66,
        "f9" => 67,
        "f10" => 68,
        "f11" => 87,
        "f12" => 88,
        "f13" => 183,
        "f14" => 184,
        "f15" => 185,
        "f16" => 186,
        "f17" => 187,
        "f18" => 188,
        "f19" => 189,
        "f20" => 190,
        "f21" => 191,
        "f22" => 192,
        "f23" => 193,
        "f24" => 194,
        _ => return None,
    }))
}

/// Parses a chord into press order, with modifiers first and the main key last.
pub fn parse_chord(chord: &str) -> Result<Vec<KeyCode>, ConfigError> {
    let trimmed = chord.trim();
    if trimmed.is_empty() {
        return Err(ConfigError::invalid("key chord is empty"));
    }

    let mut modifiers = Vec::new();
    let mut main = None;

    for raw in trimmed.split('+') {
        let token = raw.trim().to_ascii_lowercase();
        if token.is_empty() {
            continue;
        }

        if let Some(code) = modifier_code(&token) {
            if !modifiers.contains(&code) {
                modifiers.push(code);
            }
        } else if let Some(code) = key_code(&token) {
            if main.is_some() {
                return Err(ConfigError::invalid(
                    "key chord has more than one non-modifier key",
                ));
            }
            main = Some(code);
        } else {
            return Err(ConfigError::invalid(format!("unknown key '{token}'")));
        }
    }

    let main = main.ok_or_else(|| ConfigError::invalid("key chord has no non-modifier key"))?;
    modifiers.push(main);
    Ok(modifiers)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(codes: &[KeyCode]) -> Vec<u16> {
        codes.iter().map(|code| code.as_u16()).collect()
    }

    #[test]
    fn mouse_button_names_round_trip() {
        for button in MouseButton::ALL {
            assert_eq!(MouseButton::parse(button.name()), Some(button));
        }
        assert_eq!(MouseButton::parse("RIGHT"), Some(MouseButton::Right));
        assert_eq!(MouseButton::parse("eight"), None);
    }

    #[test]
    fn parses_modifiers_before_the_main_key() {
        assert_eq!(
            chord(&parse_chord("ctrl+shift+a").unwrap()),
            vec![29, 42, 30]
        );
    }

    #[test]
    fn deduplicates_repeated_modifiers() {
        assert_eq!(
            chord(&parse_chord("ctrl+ctrl+space").unwrap()),
            vec![29, 57]
        );
    }

    #[test]
    fn accepts_named_character_decimal_and_hex_keys() {
        assert_eq!(chord(&parse_chord("f24").unwrap()), vec![194]);
        // A run of digits is a raw key code first: "1" is KEY_ESC, not KEY_1.
        // Only "0" falls through to the character table (KEY_0).
        assert_eq!(chord(&parse_chord("1").unwrap()), vec![1]);
        assert_eq!(chord(&parse_chord("0").unwrap()), vec![11]);
        assert_eq!(chord(&parse_chord("0x2a").unwrap()), vec![42]);
        assert_eq!(chord(&parse_chord("-").unwrap()), vec![12]);
        assert_eq!(chord(&parse_chord("super+a").unwrap()), vec![125, 30]);
    }

    #[test]
    fn rejects_malformed_chords() {
        assert!(parse_chord("").is_err());
        assert!(parse_chord("ctrl+shift").is_err());
        assert!(parse_chord("a+b").is_err());
        assert!(parse_chord("nope").is_err());
    }
}
