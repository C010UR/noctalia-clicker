//! Human-readable preview of a normalized configuration.

use std::fmt::Write;

use crate::config::{Config, Mode, Step};
use crate::timing::RepeatTiming;

/// Describes a validated config the way `--dry-run` prints it.
pub fn describe(config: &Config) -> Result<String, serde_json::Error> {
    let mut out = String::new();
    out.push_str(&serde_json::to_string_pretty(config)?);
    out.push('\n');

    match config.mode {
        Mode::Repeat => describe_repeat(&mut out, config),
        Mode::Macro => describe_macro(&mut out, config),
        Mode::Fisher => describe_fisher(&mut out, config),
    }

    Ok(out)
}

fn describe_fisher(out: &mut String, config: &Config) {
    let fisher = &config.fisher;
    let _ = writeln!(
        out,
        "terraria fisher: cast with {} for {:.0} ms, settle {:.0} ms, bite timeout {:.1} s",
        fisher.cast_button, fisher.press_ms, fisher.settle_ms, fisher.bite_timeout_s
    );
    let _ = writeln!(
        out,
        "  detection: sensitivity {:.2}, refractory {:.0} ms, pitch variants {}, reel delay {:.0} ms",
        fisher.sensitivity, fisher.min_interval_ms, fisher.pitch_variants, fisher.reel_delay_ms
    );
    let _ = writeln!(
        out,
        "  template: {}",
        if fisher.template.is_empty() {
            "auto (Terraria Splash_5.xnb)"
        } else {
            fisher.template.as_str()
        }
    );
    let _ = writeln!(
        out,
        "  capture: {}",
        if fisher.audio_device.is_empty() {
            "default sink monitor"
        } else {
            fisher.audio_device.as_str()
        }
    );
}

fn describe_repeat(out: &mut String, config: &Config) {
    let timing = RepeatTiming::new(config.cps, config.hold_ms);
    let _ = writeln!(
        out,
        "repeat: {:.3} CPS, hold {:.3} ms, jitter {:.1}%",
        timing.effective_cps(),
        timing.effective_hold_ms(),
        config.jitter_pct
    );

    for index in 0..10 {
        let _ = writeln!(
            out,
            "  t={:8.1} ms  down {}  up +{:.1} ms",
            index as f64 * timing.effective_period_ms(),
            config.action.label(),
            timing.effective_hold_ms()
        );
    }
}

fn describe_macro(out: &mut String, config: &Config) {
    let Some(def) = &config.macro_def else {
        out.push_str("macro: none configured\n");
        return;
    };

    let repeats = if def.repeats == 0 {
        "loop".to_string()
    } else {
        def.repeats.to_string()
    };
    let _ = writeln!(
        out,
        "macro '{}': {} steps, repeat {repeats}",
        def.name,
        def.steps.len()
    );

    let mut time = 0.0;
    for (index, step) in def.steps.iter().enumerate() {
        let (label, hold, delay) = match step {
            Step::Key {
                chord,
                hold_ms,
                delay_ms,
            } => (format!("key {chord}"), *hold_ms, *delay_ms),
            Step::Mouse {
                button,
                hold_ms,
                delay_ms,
            } => (format!("mouse {button}"), *hold_ms, *delay_ms),
            Step::Delay { ms } => ("delay".to_string(), 0.0, *ms),
            Step::Scroll { direction, steps } => (format!("scroll {direction} x{steps}"), 0.0, 0.0),
        };

        let _ = writeln!(
            out,
            "  {:2}. t={:8.1} ms  {}  hold {:.1} ms  delay {:.1} ms",
            index + 1,
            time,
            label,
            hold,
            delay
        );
        time += hold + delay;
    }
    let _ = writeln!(out, "  loop delay {:.1} ms", def.loop_delay_ms);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeat_report_shows_the_effective_hold() {
        let config = Config::from_json(r#"{"cps": 30, "hold_ms": 5}"#).unwrap();
        let report = describe(&config).unwrap();

        assert!(
            report.contains("repeat: 30.000 CPS, hold 16.667 ms"),
            "{report}"
        );
        assert!(report.contains("down left"), "{report}");
    }

    #[test]
    fn macro_report_lists_steps_and_timeline() {
        let config = Config::from_json(
            r#"{
                "mode": "macro",
                "macro": {
                    "name": "demo",
                    "steps": [
                        {"type": "key", "chord": "space", "hold_ms": 10, "delay_ms": 40},
                        {"type": "delay", "ms": 100}
                    ],
                    "repeats": 2,
                    "loop_delay_ms": 250
                }
            }"#,
        )
        .unwrap();
        let report = describe(&config).unwrap();

        assert!(
            report.contains("macro 'demo': 2 steps, repeat 2"),
            "{report}"
        );
        assert!(report.contains("key space"), "{report}");
        assert!(report.contains("loop delay 250.0 ms"), "{report}");
    }
}
