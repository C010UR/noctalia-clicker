//! clickerd — input sequencer for the Noctalia Clicker plugin.
//!
//! The helper talks to `ydotoold` directly: one `AF_UNIX` `SOCK_DGRAM` socket
//! carrying raw `struct input_event` values. That avoids spawning a process per
//! click and keeps using ydotoold's existing virtual device.
//!
//! The crate is split by responsibility:
//!
//! - [`config`] parses and validates the JSON written by the plugin.
//! - [`program`] turns validated configuration into a prepared program, so the
//!   timing-critical loops never parse strings.
//! - [`engine`] owns lifecycle, signals, status reporting, and the run loops.
//! - [`input`] is the thin ydotool protocol client.
//! - [`audio`] captures the sink monitor and extracts the game's bite sound.
//! - [`detector`] matched-filters that sound, for the Terraria Fisher mode.

pub mod audio;
pub mod cli;
pub mod config;
pub mod detector;
pub mod dry_run;
pub mod engine;
pub mod error;
pub mod input;
pub mod keys;
pub mod program;
pub mod rng;
pub mod signals;
pub mod status;
pub mod time;
pub mod timing;

pub use error::Error;

/// Crate version, reported by `--version` and in `status.json`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
