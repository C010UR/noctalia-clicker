//! The helper's lifecycle and run loops.
//!
//! `Engine` owns the ydotool client, the current configuration and its prepared
//! program, and the status document. The daemon loop reacts to signals; the run
//! loops translate a program into timed input events.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use crate::audio::{self, Capture, SAMPLE_RATE};
use crate::config::{Config, Mode};
use crate::detector::Detector;
use crate::error::{ConfigError, Error, InputError};
use crate::input::YdotoolClient;
use crate::keys::KeyCode;
use crate::program::{PreparedAction, PreparedMacro, PreparedStep, PressKind, Program};
use crate::rng::Rng;
use crate::signals::{self, Signal};
use crate::status::{self, Status};
use crate::time::{self, Instant};
use crate::timing::RepeatTiming;

const STATUS_WRITE_INTERVAL: Duration = Duration::from_millis(200);
const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(3);
const DAEMON_POLL: Duration = Duration::from_millis(100);

/// How often the Terraria Fisher checks the audio feed while waiting for a bite.
const FISHER_POLL: Duration = Duration::from_millis(10);

/// Audio handed to the detector per poll; comfortably more than one poll's
/// worth so a scheduling hiccup does not lose samples.
const FISHER_TAIL_SAMPLES: usize = SAMPLE_RATE as usize / 4;

/// How long the Terraria Fisher waits for the capture process to produce audio.
const FISHER_WARMUP: Duration = Duration::from_millis(1_500);

/// Safety cap for a looping macro that sets no runtime limit.
const MACRO_LOOP_LIMIT: Duration = Duration::from_secs(3600);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StopReason {
    Stopped,
    Shutdown,
    Heartbeat,
    Duration,
    Completed,
    Error,
}

impl StopReason {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Stopped => "stopped",
            Self::Shutdown => "shutdown",
            Self::Heartbeat => "heartbeat",
            Self::Duration => "duration",
            Self::Completed => "completed",
            Self::Error => "error",
        }
    }
}

#[derive(Debug)]
pub struct Engine {
    client: Option<YdotoolClient>,
    config_path: PathBuf,
    status_path: PathBuf,
    heartbeat_path: Option<PathBuf>,
    heartbeat_started: Instant,
    last_status_write: Option<Instant>,
    config: Config,
    program: Program,
    status: Status,
    active_keys: BTreeSet<KeyCode>,
    active_buttons: BTreeSet<KeyCode>,
    rng: Rng,
}

impl Engine {
    pub fn new(
        config_path: PathBuf,
        status_path: PathBuf,
        heartbeat_path: Option<PathBuf>,
    ) -> Self {
        Self {
            client: None,
            config_path,
            status_path,
            heartbeat_path,
            heartbeat_started: Instant::now(),
            last_status_write: None,
            config: Config::default(),
            program: Program::empty(),
            status: Status::new(),
            active_keys: BTreeSet::new(),
            active_buttons: BTreeSet::new(),
            rng: Rng::new(),
        }
    }

    /// Runs the daemon until a terminating signal or a lost heartbeat.
    pub fn run(mut self) -> Result<(), Error> {
        signals::install()?;
        set_parent_death_signal();

        if let Err(err) = self.load_config() {
            // A missing config is expected until the plugin starts a run and
            // writes one. Only surface errors for a config that does exist.
            if self.config_path.exists() {
                self.status.error = err.to_string();
            }
        }

        self.status.ready = true;
        self.write_status(true);

        loop {
            match signals::next() {
                Some(Signal::Terminate | Signal::Interrupt) => {
                    self.stop(StopReason::Shutdown);
                    break;
                }
                Some(Signal::Stop) => self.stop(StopReason::Stopped),
                Some(Signal::Reload) => {
                    if let Err(err) = self.load_config() {
                        self.status.error = err.to_string();
                        self.write_status(true);
                    }
                }
                Some(Signal::Start) => {
                    let should_exit = self.start();
                    if should_exit {
                        break;
                    }
                }
                None => {}
            }

            if !self.heartbeat_ok() {
                self.stop(StopReason::Heartbeat);
                break;
            }
            self.write_status(false);
            time::sleep_for(DAEMON_POLL);
        }

        Ok(())
    }

    /// Starts a run and reports whether the daemon should exit afterwards.
    ///
    /// Configuration and connection errors are reported through `status.json`
    /// and leave the daemon alive, so a bad session never kills the helper.
    fn start(&mut self) -> bool {
        if let Err(err) = self.load_config() {
            self.report(err.to_string());
            return false;
        }
        if let Err(err) = self.client() {
            self.report(err.to_string());
            return false;
        }

        self.status.begin_run(time::unix_ms());
        self.write_status(true);

        let reason = self.run_program();
        self.stop(reason);

        matches!(reason, StopReason::Shutdown | StopReason::Heartbeat)
    }

    fn run_program(&mut self) -> StopReason {
        let result: Result<StopReason, Error> = match self.config.mode {
            Mode::Repeat => self.run_repeat().map_err(Error::from),
            Mode::Macro => self.run_macro().map_err(Error::from),
            Mode::Fisher => self.run_fisher(),
        };

        match result {
            Ok(reason) => reason,
            Err(err) => {
                self.status.error = err.to_string();
                StopReason::Error
            }
        }
    }

    fn run_repeat(&mut self) -> Result<StopReason, InputError> {
        let started = Instant::now();
        let mut next_fire = started;

        loop {
            if let Some(reason) = self.check_control() {
                return Ok(reason);
            }
            if !self.heartbeat_ok() {
                return Ok(StopReason::Heartbeat);
            }

            let timing = RepeatTiming::new(self.config.cps, self.config.hold_ms);
            self.status.cps = self.config.cps;
            self.status.hold_ms = self.config.hold_ms;
            self.status.effective_cps = timing.effective_cps();
            self.status.effective_hold_ms = timing.effective_hold_ms();
            self.status.macro_step_count = 0;

            let fire_at = next_fire.max(Instant::now());
            if let Some(reason) = self.wait_until(fire_at) {
                return Ok(reason);
            }

            let action = self.program.repeat.clone();
            self.press(&action)?;

            if let Some(reason) = self.wait_for(timing.hold()) {
                self.release(&action)?;
                return Ok(reason);
            }
            self.release(&action)?;

            self.status.clicks = self.status.clicks.saturating_add(1);
            if action.kind() == PressKind::Key {
                self.status.keys = self.status.keys.saturating_add(1);
            }
            self.write_status(false);

            if self
                .config
                .repeat_limit()
                .is_some_and(|limit| started.elapsed() >= limit)
            {
                return Ok(StopReason::Duration);
            }

            next_fire = fire_at.saturating_add(timing.jittered_period(self.jitter_factor()));
            if next_fire <= Instant::now() {
                next_fire = Instant::now().saturating_add(Duration::from_millis(1));
            }
        }
    }

    fn run_macro(&mut self) -> Result<StopReason, InputError> {
        let started = Instant::now();
        let mut repeat_index: u32 = 0;

        loop {
            if let Some(reason) = self.check_control() {
                return Ok(reason);
            }
            if !self.heartbeat_ok() {
                return Ok(StopReason::Heartbeat);
            }

            let Some(def) = self.program.macro_def.clone() else {
                self.status.error = "no macro configured".into();
                return Ok(StopReason::Error);
            };
            if def.steps.is_empty() {
                self.status.error = "macro has no steps".into();
                return Ok(StopReason::Error);
            }

            self.status.name = def.name.clone();
            self.status.macro_step_count = def.steps.len() as u32;
            self.status.macro_repeat = def.repeats;
            self.status.macro_repeat_index = repeat_index.saturating_add(1);

            for (index, step) in def.steps.iter().enumerate() {
                self.status.macro_step = index as u32 + 1;
                self.status.macro_repeat_index = repeat_index.saturating_add(1);

                if let Some(reason) = self.check_control() {
                    return Ok(reason);
                }
                if let Some(reason) = self.run_step(step)? {
                    return Ok(reason);
                }

                self.write_status(false);

                if self
                    .macro_limit(&def)
                    .is_some_and(|limit| started.elapsed() >= limit)
                {
                    return Ok(StopReason::Duration);
                }
            }

            repeat_index = repeat_index.saturating_add(1);
            if def.repeats != 0 && repeat_index >= def.repeats {
                return Ok(StopReason::Completed);
            }
            if let Some(reason) = self.wait_for(def.loop_delay) {
                return Ok(reason);
            }
        }
    }

    /// Cast, listen for the game's bite sound, reel in, repeat.
    ///
    /// Detection runs on a matched filter over the sink monitor, so the loop
    /// never looks at the screen. All clicks go through the same tracked
    /// press/release path as the clicker, which keeps the release-on-stop
    /// guarantee intact.
    fn run_fisher(&mut self) -> Result<StopReason, Error> {
        let settings = self.config.fisher.clone();
        let template = audio::load_template(&settings)?;
        self.status.template_source = template.source.clone();
        self.status.fisher_state = "starting".into();
        self.write_status(true);

        let capture = Capture::start(&settings.audio_device)?;
        let mut detector = Detector::new(
            &template.samples,
            settings.sensitivity as f32,
            time::millis(settings.min_interval_ms),
            settings.pitch_variants,
        );

        self.wait_for_capture(&capture)?;

        let started = Instant::now();
        loop {
            if let Some(reason) = self.check_control() {
                return Ok(reason);
            }
            if !self.heartbeat_ok() {
                return Ok(StopReason::Heartbeat);
            }

            // Re-read every pass so panel changes land on the next cast.
            let settings = self.config.fisher.clone();
            let action = PreparedAction::Mouse(settings.cast_button);

            self.status.fisher_state = "casting".into();
            if let Some(reason) = self.fish_tap(&action, settings.press_ms)? {
                return Ok(reason);
            }
            self.status.fisher_casts = self.status.fisher_casts.saturating_add(1);
            self.write_status(false);

            if let Some(reason) = self.wait_for(time::millis(settings.settle_ms)) {
                return Ok(reason);
            }

            let snapshot = capture.snapshot(FISHER_TAIL_SAMPLES);
            detector.reset(snapshot.total);
            self.status.fisher_state = "waiting".into();
            self.write_status(false);

            let timeout = time::millis(settings.bite_timeout_s * 1_000.0);
            let deadline = Instant::now().saturating_add(timeout);
            let mut bite = false;
            while Instant::now() < deadline {
                if let Some(reason) = self.check_control() {
                    return Ok(reason);
                }
                if !self.heartbeat_ok() {
                    return Ok(StopReason::Heartbeat);
                }

                let snapshot = capture.snapshot(FISHER_TAIL_SAMPLES);
                if snapshot.closed {
                    if let Some(error) = snapshot.error {
                        return Err(crate::error::AudioError::Capture(error).into());
                    }
                    return Err(crate::error::AudioError::Capture(
                        "the capture process stopped".to_string(),
                    )
                    .into());
                }

                let reading = detector.poll(&snapshot.samples, snapshot.total, Instant::now());
                self.status.audio_level = reading.level;
                self.status.match_score = reading.score;
                if reading.detected {
                    bite = true;
                    break;
                }

                self.write_status(false);
                time::sleep_for(FISHER_POLL);
            }

            if bite {
                self.status.fisher_state = "reeling".into();
                if let Some(reason) = self.fish_tap(&action, settings.press_ms)? {
                    return Ok(reason);
                }
                self.status.fisher_bites = self.status.fisher_bites.saturating_add(1);
                if let Some(reason) = self.wait_for(time::millis(settings.post_reel_ms)) {
                    return Ok(reason);
                }
            } else {
                // The line is still out: reel it in, then cast again.
                self.status.fisher_misses = self.status.fisher_misses.saturating_add(1);
                self.status.fisher_state = "recovering".into();
                if let Some(reason) = self.fish_tap(&action, settings.press_ms)? {
                    return Ok(reason);
                }
                if let Some(reason) = self.wait_for(time::millis(settings.recover_ms)) {
                    return Ok(reason);
                }
            }

            self.write_status(false);
            if settings.max_casts > 0 && self.status.fisher_casts >= settings.max_casts {
                return Ok(StopReason::Completed);
            }
            if self
                .config
                .repeat_limit()
                .is_some_and(|limit| started.elapsed() >= limit)
            {
                return Ok(StopReason::Duration);
            }
        }
    }

    /// Presses and releases one button, keeping the release guarantee when a
    /// stop signal arrives mid-press.
    fn fish_tap(
        &mut self,
        action: &PreparedAction,
        press_ms: f64,
    ) -> Result<Option<StopReason>, InputError> {
        self.press(action)?;
        let reason = self.wait_for(time::millis(press_ms));
        self.release(action)?;
        Ok(reason)
    }

    /// Fails fast when the capture device is wrong instead of casting forever.
    fn wait_for_capture(&mut self, capture: &Capture) -> Result<(), Error> {
        let deadline = Instant::now().saturating_add(FISHER_WARMUP);
        loop {
            let snapshot = capture.snapshot(64);
            if snapshot.total > 0 {
                return Ok(());
            }
            if snapshot.closed {
                let message = snapshot
                    .error
                    .unwrap_or_else(|| "the capture device produced no audio".to_string());
                return Err(crate::error::AudioError::Capture(message).into());
            }
            if Instant::now() >= deadline {
                return Err(crate::error::AudioError::Capture(
                    "no audio arrived from the capture device".to_string(),
                )
                .into());
            }
            time::sleep_for(Duration::from_millis(20));
        }
    }

    fn run_step(&mut self, step: &PreparedStep) -> Result<Option<StopReason>, InputError> {
        match step {
            PreparedStep::Press {
                action,
                hold,
                delay,
            } => {
                self.press(action)?;
                if let Some(reason) = self.wait_for(*hold) {
                    self.release(action)?;
                    return Ok(Some(reason));
                }
                self.release(action)?;

                self.status.clicks = self.status.clicks.saturating_add(1);
                if action.kind() == PressKind::Key {
                    self.status.keys = self.status.keys.saturating_add(1);
                }
                Ok(self.wait_for(*delay))
            }
            PreparedStep::Delay(duration) => Ok(self.wait_for(*duration)),
            PreparedStep::Scroll { direction, steps } => {
                self.wheel(direction.sign() * *steps)?;
                self.write_status(false);
                Ok(None)
            }
        }
    }

    /// Runtime limit for a macro: session limit, macro limit, then a safety cap
    /// for looping macros.
    fn macro_limit(&self, def: &PreparedMacro) -> Option<Duration> {
        self.config.repeat_limit().or_else(|| {
            def.max_runtime
                .or_else(|| (def.repeats == 0).then_some(MACRO_LOOP_LIMIT))
        })
    }

    fn jitter_factor(&mut self) -> f64 {
        let jitter = self.config.jitter_pct / 100.0;
        if jitter <= 0.0 {
            1.0
        } else {
            1.0 + (self.rng.next_f64() * 2.0 - 1.0) * jitter
        }
    }

    /// Sleeps until `deadline`, returning early when a stop signal arrives.
    fn wait_until(&mut self, deadline: Instant) -> Option<StopReason> {
        loop {
            if Instant::now() >= deadline {
                return self.check_control();
            }
            time::sleep_until(deadline);
            if let Some(reason) = self.check_control() {
                return Some(reason);
            }
            if !self.heartbeat_ok() {
                return Some(StopReason::Heartbeat);
            }
        }
    }

    fn wait_for(&mut self, duration: Duration) -> Option<StopReason> {
        self.wait_until(Instant::now().saturating_add(duration))
    }

    fn check_control(&mut self) -> Option<StopReason> {
        match signals::take_control() {
            Some(Signal::Terminate | Signal::Interrupt) => Some(StopReason::Shutdown),
            Some(Signal::Stop) => Some(StopReason::Stopped),
            Some(Signal::Reload) => {
                if let Err(err) = self.load_config() {
                    self.status.error = err.to_string();
                }
                None
            }
            _ => None,
        }
    }

    fn load_config(&mut self) -> Result<(), ConfigError> {
        let config = Config::load(&self.config_path)?;
        let program = Program::new(&config)?;

        self.status.mode = config.mode.as_str().to_string();
        self.status.name = config
            .macro_def
            .as_ref()
            .map(|def| def.name.clone())
            .unwrap_or_default();
        self.status.cps = config.cps;
        self.status.hold_ms = config.hold_ms;
        self.status.jitter_pct = config.jitter_pct;
        self.status.action = match config.mode {
            Mode::Fisher => format!("fish:{}", config.fisher.cast_button),
            _ => config.action.label(),
        };
        self.status.error.clear();

        self.config = config;
        self.program = program;
        Ok(())
    }

    fn heartbeat_ok(&self) -> bool {
        let Some(path) = &self.heartbeat_path else {
            return true;
        };
        match fs::metadata(path).and_then(|meta| meta.modified()) {
            Ok(modified) => SystemTime::now()
                .duration_since(modified)
                .is_ok_and(|age| age < HEARTBEAT_TIMEOUT),
            // The plugin may not have written a heartbeat yet; allow a grace
            // period after startup instead of failing immediately.
            Err(_) => self.heartbeat_started.elapsed() < HEARTBEAT_TIMEOUT,
        }
    }

    fn client(&mut self) -> Result<&YdotoolClient, InputError> {
        if self.client.is_none() {
            let client = YdotoolClient::connect()?;
            self.client = Some(client);
        }
        // The branch above guarantees a client; this cannot be reached empty.
        Ok(self.client.as_ref().expect("client is initialized"))
    }

    fn press(&mut self, action: &PreparedAction) -> Result<(), InputError> {
        match action {
            PreparedAction::Mouse(button) => {
                let code = button.code();
                if self.active_buttons.insert(code) {
                    self.send_key(code, true)?;
                }
            }
            PreparedAction::Chord(codes) => {
                for &code in codes {
                    if self.active_keys.insert(code) {
                        self.send_key(code, true)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn release(&mut self, action: &PreparedAction) -> Result<(), InputError> {
        match action {
            PreparedAction::Mouse(button) => {
                let code = button.code();
                if self.active_buttons.remove(&code) {
                    self.send_key(code, false)?;
                }
            }
            PreparedAction::Chord(codes) => {
                for &code in codes.iter().rev() {
                    if self.active_keys.remove(&code) {
                        self.send_key(code, false)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn send_key(&mut self, code: KeyCode, pressed: bool) -> Result<(), InputError> {
        self.client()?.key(code, pressed)
    }

    fn wheel(&mut self, steps: i32) -> Result<(), InputError> {
        self.client()?.wheel(steps)
    }

    /// Releases everything the helper may still be holding. Best effort: this
    /// also runs on shutdown, when the socket may already be gone.
    fn release_all(&mut self) {
        let keys: Vec<KeyCode> = self.active_keys.iter().rev().copied().collect();
        let buttons: Vec<KeyCode> = self.active_buttons.iter().rev().copied().collect();
        self.active_keys.clear();
        self.active_buttons.clear();

        if let Some(client) = &self.client {
            for code in buttons.into_iter().chain(keys) {
                let _ = client.key(code, false);
            }
        }
    }

    fn stop(&mut self, reason: StopReason) {
        self.release_all();
        self.status.running = false;
        self.status.reason = reason.as_str().to_string();
        self.status.macro_step = 0;
        self.status.macro_step_count = 0;
        self.status.macro_repeat_index = 0;
        self.status.fisher_state = "idle".into();
        self.write_status(true);
    }

    fn report(&mut self, message: String) {
        self.status.error = message;
        self.write_status(true);
    }

    fn write_status(&mut self, force: bool) {
        if !force {
            if let Some(last) = self.last_status_write {
                if last.elapsed() < STATUS_WRITE_INTERVAL {
                    return;
                }
            }
        }

        self.last_status_write = Some(Instant::now());
        self.status.updated_at_ms = time::unix_ms();
        self.status.pid = std::process::id();
        let _ = status::write_atomic(&self.status_path, &self.status);
    }
}

/// Asks the kernel to terminate the helper when the plugin's service exits.
fn set_parent_death_signal() {
    // Best effort: the plugin also sends SIGTERM on exit and the helper stays
    // useful without this.
    // SAFETY: `prctl` is called with `PR_SET_PDEATHSIG` and a valid signal.
    let _ = unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_reasons_match_the_plugin_protocol() {
        assert_eq!(StopReason::Stopped.as_str(), "stopped");
        assert_eq!(StopReason::Shutdown.as_str(), "shutdown");
        assert_eq!(StopReason::Heartbeat.as_str(), "heartbeat");
        assert_eq!(StopReason::Duration.as_str(), "duration");
        assert_eq!(StopReason::Completed.as_str(), "completed");
        assert_eq!(StopReason::Error.as_str(), "error");
    }
}
