//! CLI entry point.
//!
//! `main` only translates arguments into a [`cli::Command`] and maps failures
//! to exit codes: 2 for a bad invocation, 1 for a runtime error.

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clickerd::audio;
use clickerd::cli::{self, Command};
use clickerd::config::Config;
use clickerd::detector::Detector;
use clickerd::engine::Engine;
use clickerd::input::YdotoolClient;
use clickerd::keys::{MouseButton, SAFETY_RELEASE};
use clickerd::time;
use clickerd::VERSION;

/// Press duration for `--click-once`; long enough for per-frame games to see it.
const SINGLE_CLICK_HOLD: Duration = Duration::from_millis(8);

fn main() -> ExitCode {
    let command = match cli::parse(std::env::args().skip(1)) {
        Ok(command) => command,
        Err(err) => {
            eprintln!("clickerd: {err}");
            eprintln!("Try 'clickerd --help' for usage.");
            return ExitCode::from(2);
        }
    };

    match execute(command) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("clickerd: {err}");
            ExitCode::FAILURE
        }
    }
}

fn execute(command: Command) -> Result<(), clickerd::Error> {
    match command {
        Command::Help => {
            println!("{}", cli::help());
            Ok(())
        }
        Command::Version => {
            println!("clickerd {VERSION}");
            Ok(())
        }
        Command::ReleaseAll => release_all(),
        Command::ClickOnce(button) => click_once(button),
        Command::DryRun(path) => {
            let config = Config::load(&path)?;
            print!("{}", clickerd::dry_run::describe(&config)?);
            Ok(())
        }
        Command::Daemon(options) => {
            Engine::new(options.config, options.status, options.heartbeat).run()
        }
        Command::ProbeAudio { config, seconds } => probe_audio(config, seconds),
    }
}

/// Prints the Terraria Fisher's live audio level and match score without touching input.
/// Useful when tuning sensitivity or when no bites are detected.
fn probe_audio(path: PathBuf, seconds: u64) -> Result<(), clickerd::Error> {
    let config = Config::load(&path)?;
    let settings = &config.fisher;
    let template = audio::load_template(settings)?;
    println!(
        "template: {} ({} samples @ {} Hz)",
        template.source,
        template.samples.len(),
        template.sample_rate
    );

    let capture = audio::Capture::start(&settings.audio_device)?;
    let mut detector = Detector::new(
        &template.samples,
        settings.sensitivity as f32,
        time::millis(settings.min_interval_ms),
        settings.pitch_variants,
    );
    println!(
        "listening for {} s, threshold {:.2}. Cast a line in the game.",
        seconds, settings.sensitivity
    );

    let deadline = time::Instant::now().saturating_add(Duration::from_secs(seconds));
    let mut last_print = time::Instant::now();
    let mut stdout = std::io::stdout();

    while time::Instant::now() < deadline {
        let snapshot = capture.snapshot(audio::SAMPLE_RATE as usize / 4);
        let reading = detector.poll(&snapshot.samples, snapshot.total, time::Instant::now());
        if last_print.elapsed() >= Duration::from_millis(100) {
            last_print = time::Instant::now();
            let meters = (reading.level * 24.0).round() as usize;
            let bar = "|".repeat(meters);
            let _ = write!(
                stdout,
                "\r{:<24} level {:3.0}%  score {:.2}{}",
                bar,
                reading.level * 100.0,
                reading.score,
                if reading.detected { "  BITE" } else { "      " }
            );
            let _ = stdout.flush();
        }
        time::sleep_for(Duration::from_millis(10));
    }
    println!();
    Ok(())
}

fn release_all() -> Result<(), clickerd::Error> {
    let client = YdotoolClient::connect()?;
    for code in SAFETY_RELEASE {
        let _ = client.key(code, false);
    }
    println!("released");
    Ok(())
}

fn click_once(button: MouseButton) -> Result<(), clickerd::Error> {
    let client = YdotoolClient::connect()?;
    client.key(button.code(), true)?;
    time::sleep_for(SINGLE_CLICK_HOLD);
    client.key(button.code(), false)?;
    println!("clicked {button}");
    Ok(())
}
