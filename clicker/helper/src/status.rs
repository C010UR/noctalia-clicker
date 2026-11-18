//! `status.json`, the only channel back to the plugin.
//!
//! Field names are part of the plugin protocol: `service.luau` reads them
//! directly. Writes go through a temporary file and a rename so the plugin
//! never observes a partial document.

use std::fs;
use std::io;
use std::path::Path;

use serde::Serialize;

use crate::VERSION;

#[derive(Debug, Serialize)]
pub struct Status {
    pub version: String,
    pub pid: u32,
    pub ready: bool,
    pub running: bool,
    pub mode: String,
    pub name: String,
    pub action: String,
    pub cps: f64,
    pub effective_cps: f64,
    pub hold_ms: f64,
    pub effective_hold_ms: f64,
    pub jitter_pct: f64,
    pub clicks: u64,
    pub keys: u64,
    pub macro_step: u32,
    pub macro_step_count: u32,
    pub macro_repeat: u32,
    pub macro_repeat_index: u32,
    pub fisher_state: String,
    pub fisher_casts: u32,
    pub fisher_bites: u32,
    pub fisher_misses: u32,
    pub audio_level: f32,
    pub match_score: f32,
    pub template_source: String,
    pub started_at_ms: u64,
    pub updated_at_ms: u64,
    pub reason: String,
    pub error: String,
}

impl Status {
    pub fn new() -> Self {
        Self {
            version: VERSION.to_string(),
            pid: std::process::id(),
            ready: false,
            running: false,
            mode: "repeat".into(),
            name: String::new(),
            action: "left".into(),
            cps: crate::config::DEFAULT_CPS,
            effective_cps: 0.0,
            hold_ms: crate::config::DEFAULT_HOLD_MS,
            effective_hold_ms: 0.0,
            jitter_pct: 0.0,
            clicks: 0,
            keys: 0,
            macro_step: 0,
            macro_step_count: 0,
            macro_repeat: 0,
            macro_repeat_index: 0,
            fisher_state: "idle".into(),
            fisher_casts: 0,
            fisher_bites: 0,
            fisher_misses: 0,
            audio_level: 0.0,
            match_score: 0.0,
            template_source: String::new(),
            started_at_ms: 0,
            updated_at_ms: 0,
            reason: String::new(),
            error: String::new(),
        }
    }

    /// Clears the per-run counters while keeping the configured values.
    pub fn begin_run(&mut self, started_at_ms: u64) {
        self.running = true;
        self.reason.clear();
        self.error.clear();
        self.clicks = 0;
        self.keys = 0;
        self.started_at_ms = started_at_ms;
        self.macro_step = 0;
        self.macro_step_count = 0;
        self.macro_repeat = 0;
        self.macro_repeat_index = 0;
        self.fisher_state = "idle".into();
        self.fisher_casts = 0;
        self.fisher_bites = 0;
        self.fisher_misses = 0;
        self.audio_level = 0.0;
        self.match_score = 0.0;
    }
}

impl Default for Status {
    fn default() -> Self {
        Self::new()
    }
}

/// Serializes and atomically replaces `path`.
pub fn write_atomic(path: &Path, status: &Status) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let data = serde_json::to_vec_pretty(status).map_err(io::Error::other)?;
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, data)?;
    fs::rename(temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_uses_the_field_names_the_plugin_reads() {
        let json = serde_json::to_value(Status::new()).unwrap();
        for key in [
            "version",
            "pid",
            "ready",
            "running",
            "mode",
            "name",
            "action",
            "cps",
            "effective_cps",
            "hold_ms",
            "effective_hold_ms",
            "jitter_pct",
            "clicks",
            "keys",
            "macro_step",
            "macro_step_count",
            "macro_repeat",
            "macro_repeat_index",
            "fisher_state",
            "fisher_casts",
            "fisher_bites",
            "fisher_misses",
            "audio_level",
            "match_score",
            "template_source",
            "started_at_ms",
            "updated_at_ms",
            "reason",
            "error",
        ] {
            assert!(json.get(key).is_some(), "missing {key}");
        }
    }

    #[test]
    fn begin_run_resets_counters_only() {
        let mut status = Status::new();
        status.cps = 30.0;
        status.clicks = 12;
        status.error = "boom".into();
        status.reason = "stopped".into();

        status.begin_run(42);

        assert!(status.running);
        assert_eq!(status.clicks, 0);
        assert!(status.error.is_empty());
        assert!(status.reason.is_empty());
        assert_eq!(status.started_at_ms, 42);
        assert_eq!(status.cps, 30.0);
    }

    #[test]
    fn writes_atomically() {
        let directory =
            std::env::temp_dir().join(format!("clickerd-status-{}", std::process::id()));
        let path = directory.join("status.json");
        write_atomic(&path, &Status::new()).unwrap();

        let raw = fs::read_to_string(&path).unwrap();
        assert!(raw.contains("\"ready\": false"), "{raw}");
        assert!(!path.with_extension("json.tmp").exists());

        let _ = fs::remove_dir_all(directory);
    }
}
