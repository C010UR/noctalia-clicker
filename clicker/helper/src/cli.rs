//! Command line parsing.
//!
//! Parsing is separated from execution so every mode is easy to unit test and
//! `main` stays a thin dispatcher.

use std::path::PathBuf;

use crate::error::CliError;
use crate::keys::MouseButton;
use crate::VERSION;

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Daemon(DaemonOptions),
    DryRun(PathBuf),
    ClickOnce(MouseButton),
    ReleaseAll,
    ProbeAudio { config: PathBuf, seconds: u64 },
    Help,
    Version,
}

#[derive(Debug, PartialEq, Eq)]
pub struct DaemonOptions {
    pub config: PathBuf,
    pub status: PathBuf,
    pub heartbeat: Option<PathBuf>,
}

/// Parses arguments, excluding the program name.
pub fn parse<I>(args: I) -> Result<Command, CliError>
where
    I: IntoIterator<Item = String>,
{
    let mut args = args.into_iter();
    let mut daemon = false;
    let mut dry_run = false;
    let mut release_all = false;
    let mut help = false;
    let mut version = false;
    let mut click_once: Option<String> = None;
    let mut probe: Option<String> = None;
    let mut config: Option<PathBuf> = None;
    let mut status: Option<PathBuf> = None;
    let mut heartbeat: Option<PathBuf> = None;

    while let Some(argument) = args.next() {
        match argument.as_str() {
            "-h" | "--help" => help = true,
            "-V" | "--version" => version = true,
            "--daemon" => daemon = true,
            "--dry-run" => dry_run = true,
            "--release-all" => release_all = true,
            "--click-once" => click_once = Some(value(&mut args, "--click-once")?),
            "--probe-audio" => probe = Some(value(&mut args, "--probe-audio")?),
            "--config" => config = Some(PathBuf::from(value(&mut args, "--config")?)),
            "--status" => status = Some(PathBuf::from(value(&mut args, "--status")?)),
            "--heartbeat" => heartbeat = Some(PathBuf::from(value(&mut args, "--heartbeat")?)),
            other => return Err(CliError::UnknownArgument(other.to_string())),
        }
    }

    if help {
        return Ok(Command::Help);
    }
    if version {
        return Ok(Command::Version);
    }
    if release_all {
        return Ok(Command::ReleaseAll);
    }
    if let Some(button) = click_once {
        let button = MouseButton::parse(&button).ok_or(CliError::UnknownButton(button))?;
        return Ok(Command::ClickOnce(button));
    }
    if let Some(seconds) = probe {
        let config = config.ok_or(CliError::MissingOption("--config"))?;
        let seconds = seconds
            .parse::<u64>()
            .map_err(|_| CliError::UnknownValue("--probe-audio", seconds.clone()))?
            .clamp(1, 3_600);
        return Ok(Command::ProbeAudio { config, seconds });
    }
    if dry_run {
        let config = config.ok_or(CliError::MissingOption("--config"))?;
        return Ok(Command::DryRun(config));
    }
    if daemon {
        return Ok(Command::Daemon(DaemonOptions {
            config: config.ok_or(CliError::MissingOption("--config"))?,
            status: status.ok_or(CliError::MissingOption("--status"))?,
            heartbeat,
        }));
    }

    Err(CliError::MissingMode)
}

fn value<I>(args: &mut I, flag: &'static str) -> Result<String, CliError>
where
    I: Iterator<Item = String>,
{
    args.next().ok_or(CliError::MissingValue(flag))
}

pub fn help() -> String {
    format!(
        "clickerd {VERSION}
Input sequencer for the Noctalia Clicker plugin.

Usage:
  clickerd --daemon --config PATH --status PATH [--heartbeat PATH]
  clickerd --dry-run --config PATH
  clickerd --probe-audio SECONDS --config PATH
  clickerd --click-once BUTTON
  clickerd --release-all
  clickerd --version

Options:
  --daemon        run as the long-lived helper
  --dry-run       validate and describe a config, then exit
  --probe-audio   print the Terraria Fisher's audio level and match score, no clicks
  --click-once    press and release one mouse button
  --release-all   release every button and key the helper may hold
  -h, --help      show this help
  -V, --version   show the version

Signals:
  SIGHUP   reload config
  SIGUSR1  start
  SIGUSR2  stop
  SIGTERM  stop and exit"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_args(args: &[&str]) -> Result<Command, CliError> {
        parse(args.iter().map(|argument| (*argument).to_string()))
    }

    #[test]
    fn parses_the_daemon_invocation_the_plugin_uses() {
        let command = parse_args(&[
            "--daemon",
            "--config",
            "/run/config.json",
            "--status",
            "/run/status.json",
            "--heartbeat",
            "/run/heartbeat",
        ]);

        assert_eq!(
            command.unwrap(),
            Command::Daemon(DaemonOptions {
                config: PathBuf::from("/run/config.json"),
                status: PathBuf::from("/run/status.json"),
                heartbeat: Some(PathBuf::from("/run/heartbeat")),
            })
        );
    }

    #[test]
    fn parses_short_lived_modes() {
        assert_eq!(parse_args(&["--version"]).unwrap(), Command::Version);
        assert_eq!(parse_args(&["--release-all"]).unwrap(), Command::ReleaseAll);
        assert_eq!(
            parse_args(&["--dry-run", "--config", "c.json"]).unwrap(),
            Command::DryRun(PathBuf::from("c.json"))
        );
        assert_eq!(
            parse_args(&["--click-once", "RIGHT"]).unwrap(),
            Command::ClickOnce(MouseButton::Right)
        );
        assert_eq!(
            parse_args(&["--probe-audio", "10", "--config", "c.json"]).unwrap(),
            Command::ProbeAudio {
                config: PathBuf::from("c.json"),
                seconds: 10,
            }
        );
    }

    #[test]
    fn help_and_version_win_over_other_modes() {
        assert_eq!(parse_args(&["--daemon", "--help"]).unwrap(), Command::Help);
    }

    #[test]
    fn rejects_incomplete_and_unknown_arguments() {
        assert_eq!(
            parse_args(&["--daemon", "--config"]).unwrap_err(),
            CliError::MissingValue("--config")
        );
        assert_eq!(
            parse_args(&["--daemon", "--config", "c.json"]).unwrap_err(),
            CliError::MissingOption("--status")
        );
        assert_eq!(
            parse_args(&["--click-once", "turbo"]).unwrap_err(),
            CliError::UnknownButton("turbo".to_string())
        );
        assert_eq!(
            parse_args(&["--nope"]).unwrap_err(),
            CliError::UnknownArgument("--nope".to_string())
        );
        assert_eq!(
            parse_args(&["--config", "c.json"]).unwrap_err(),
            CliError::MissingMode
        );
    }
}
