//! clickerd - input sequencer for the Noctalia Clicker plugin.
//!
//! It speaks ydotool's daemon protocol directly: one AF_UNIX SOCK_DGRAM
//! connection carrying raw `struct input_event` values. No per-click process,
//! no private virtual device, and the existing ydotoold device stays in use.

use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::io;
use std::os::unix::io::RawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const EV_REL: u16 = 2;
const SYN_REPORT: u16 = 0;
const REL_WHEEL: u16 = 8;

static SIG_HUP: AtomicBool = AtomicBool::new(false);
static SIG_USR1: AtomicBool = AtomicBool::new(false);
static SIG_USR2: AtomicBool = AtomicBool::new(false);
static SIG_TERM: AtomicBool = AtomicBool::new(false);
static SIG_INT: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_signal(sig: libc::c_int) {
    match sig {
        libc::SIGHUP => {
            SIG_HUP.store(true, Ordering::SeqCst);
        }
        libc::SIGUSR1 => {
            SIG_USR1.store(true, Ordering::SeqCst);
        }
        libc::SIGUSR2 => {
            SIG_USR2.store(true, Ordering::SeqCst);
        }
        libc::SIGTERM => {
            SIG_TERM.store(true, Ordering::SeqCst);
        }
        libc::SIGINT => {
            SIG_INT.store(true, Ordering::SeqCst);
        }
        _ => {}
    }
}

fn install_signals() {
    for sig in [
        libc::SIGHUP,
        libc::SIGUSR1,
        libc::SIGUSR2,
        libc::SIGTERM,
        libc::SIGINT,
    ] {
        unsafe {
            let mut sa: libc::sigaction = std::mem::zeroed();
            sa.sa_sigaction = handle_signal as *const () as usize;
            sa.sa_flags = 0;
            libc::sigemptyset(&mut sa.sa_mask);
            libc::sigaction(sig, &sa, std::ptr::null_mut());
        }
    }
}

fn take(flag: &AtomicBool) -> bool {
    flag.swap(false, Ordering::SeqCst)
}

fn now_ns() -> u64 {
    let mut ts: libc::timespec = unsafe { std::mem::zeroed() };
    unsafe {
        libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts);
    }
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn sleep_until(deadline_ns: u64) {
    let ts = libc::timespec {
        tv_sec: (deadline_ns / 1_000_000_000) as libc::time_t,
        tv_nsec: (deadline_ns % 1_000_000_000) as libc::c_long,
    };
    unsafe {
        libc::clock_nanosleep(
            libc::CLOCK_MONOTONIC,
            libc::TIMER_ABSTIME,
            &ts,
            std::ptr::null_mut(),
        );
    }
}

fn sleep_ms(ms: f64) {
    if ms > 0.0 {
        sleep_until(now_ns() + (ms * 1_000_000.0) as u64);
    }
}

struct Rng(u64);

impl Rng {
    fn new() -> Self {
        Self(now_ns() ^ ((std::process::id() as u64) << 32) | 0x9e3779b97f4a7c15)
    }

    fn next_f64(&mut self) -> f64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        ((x >> 11) as f64) / ((1u64 << 53) as f64)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Mode {
    Repeat,
    Macro,
}

impl Default for Mode {
    fn default() -> Self {
        Mode::Repeat
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Action {
    Mouse {
        #[serde(default = "default_button")]
        button: String,
    },
    Key { chord: String },
}

impl Default for Action {
    fn default() -> Self {
        Action::Mouse {
            button: default_button(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct MacroDef {
    #[serde(default)]
    name: String,
    steps: Vec<Step>,
    #[serde(default = "default_repeat", alias = "repeat")]
    repeats: u32,
    #[serde(default)]
    loop_delay_ms: f64,
    #[serde(default)]
    max_runtime_s: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Step {
    Key {
        chord: String,
        #[serde(default)]
        hold_ms: f64,
        #[serde(default)]
        delay_ms: f64,
    },
    Mouse {
        #[serde(default = "default_button")]
        button: String,
        #[serde(default)]
        hold_ms: f64,
        #[serde(default)]
        delay_ms: f64,
    },
    Delay {
        ms: f64,
    },
    Scroll {
        #[serde(default = "default_direction")]
        direction: String,
        #[serde(default = "default_scroll_steps")]
        steps: i32,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct Config {
    #[serde(default)]
    mode: Mode,
    #[serde(default = "default_cps")]
    cps: f64,
    #[serde(default = "default_hold")]
    hold_ms: f64,
    #[serde(default)]
    jitter_pct: f64,
    #[serde(default)]
    max_runtime_s: f64,
    #[serde(default)]
    action: Action,
    #[serde(default, rename = "macro")]
    macro_def: Option<MacroDef>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            mode: Mode::Repeat,
            cps: default_cps(),
            hold_ms: default_hold(),
            jitter_pct: 0.0,
            max_runtime_s: 0.0,
            action: Action::default(),
            macro_def: None,
        }
    }
}

fn default_button() -> String {
    "left".to_string()
}

fn default_direction() -> String {
    "down".to_string()
}

fn default_scroll_steps() -> i32 {
    1
}

fn default_cps() -> f64 {
    12.0
}

fn default_hold() -> f64 {
    8.0
}

fn default_repeat() -> u32 {
    1
}

fn finite(value: f64) -> bool {
    value.is_finite()
}

impl Config {
    fn normalized(self) -> Result<Self, String> {
        if !finite(self.cps) || self.cps <= 0.0 {
            return Err("cps must be a positive number".into());
        }
        if !finite(self.hold_ms) || self.hold_ms < 0.0 {
            return Err("hold_ms must be zero or positive".into());
        }
        if !finite(self.jitter_pct) || !(0.0..=50.0).contains(&self.jitter_pct) {
            return Err("jitter_pct must be between 0 and 50".into());
        }
        if !finite(self.max_runtime_s) || !(0.0..=86400.0).contains(&self.max_runtime_s) {
            return Err("max_runtime_s must be between 0 and 86400".into());
        }

        let mut cfg = self;
        cfg.cps = cfg.cps.clamp(1.0, 1000.0);
        cfg.hold_ms = cfg.hold_ms.clamp(0.0, 2000.0);
        cfg.jitter_pct = cfg.jitter_pct.clamp(0.0, 50.0);
        cfg.max_runtime_s = cfg.max_runtime_s.clamp(0.0, 86400.0);

        match &cfg.action {
            Action::Mouse { button } => {
                mouse_code(button)?;
            }
            Action::Key { chord } => {
                parse_chord(chord)?;
            }
        }

        if let Some(def) = &cfg.macro_def {
            if def.steps.is_empty() {
                return Err("macro has no steps".into());
            }
            if def.steps.len() > 256 {
                return Err("macro has more than 256 steps".into());
            }
            if def.repeats > 1_000_000 {
                return Err("macro repeat is too large".into());
            }
            if !finite(def.loop_delay_ms) || !(0.0..=60000.0).contains(&def.loop_delay_ms) {
                return Err("macro loop_delay_ms must be between 0 and 60000".into());
            }
            if !finite(def.max_runtime_s) || !(0.0..=86400.0).contains(&def.max_runtime_s) {
                return Err("macro max_runtime_s must be between 0 and 86400".into());
            }
            for (index, step) in def.steps.iter().enumerate() {
                validate_step(step).map_err(|err| format!("step {}: {}", index + 1, err))?;
            }
        }

        if cfg.mode == Mode::Macro && cfg.macro_def.is_none() {
            return Err("mode is macro but no macro is configured".into());
        }

        Ok(cfg)
    }
}

fn validate_step(step: &Step) -> Result<(), String> {
    match step {
        Step::Key {
            chord,
            hold_ms,
            delay_ms,
        } => {
            parse_chord(chord)?;
            if !finite(*hold_ms) || !(0.0..=2000.0).contains(hold_ms) {
                return Err("hold_ms must be between 0 and 2000".into());
            }
            if !finite(*delay_ms) || !(0.0..=60000.0).contains(delay_ms) {
                return Err("delay_ms must be between 0 and 60000".into());
            }
        }
        Step::Mouse {
            button,
            hold_ms,
            delay_ms,
        } => {
            mouse_code(button)?;
            if !finite(*hold_ms) || !(0.0..=2000.0).contains(hold_ms) {
                return Err("hold_ms must be between 0 and 2000".into());
            }
            if !finite(*delay_ms) || !(0.0..=60000.0).contains(delay_ms) {
                return Err("delay_ms must be between 0 and 60000".into());
            }
        }
        Step::Delay { ms } => {
            if !finite(*ms) || !(0.0..=60000.0).contains(ms) {
                return Err("delay ms must be between 0 and 60000".into());
            }
        }
        Step::Scroll { direction, steps } => {
            let direction = direction.to_ascii_lowercase();
            if direction != "up" && direction != "down" {
                return Err("scroll direction must be up or down".into());
            }
            if !(1..=10).contains(steps) {
                return Err("scroll steps must be between 1 and 10".into());
            }
        }
    }
    Ok(())
}

fn mouse_code(button: &str) -> Result<u16, String> {
    match button.to_ascii_lowercase().as_str() {
        "left" => Ok(272),
        "right" => Ok(273),
        "middle" => Ok(274),
        "side" => Ok(275),
        "extra" => Ok(276),
        "forward" => Ok(277),
        "back" => Ok(278),
        "task" => Ok(279),
        other => Err(format!("unknown mouse button '{other}'")),
    }
}

fn modifier_code(token: &str) -> Option<u16> {
    match token {
        "ctrl" | "control" => Some(29),
        "shift" => Some(42),
        "alt" => Some(56),
        "super" | "meta" | "win" | "logo" => Some(125),
        _ => None,
    }
}

fn key_code(token: &str) -> Option<u16> {
    let token = token.to_ascii_lowercase();
    if let Some(hex) = token.strip_prefix("0x") {
        return u16::from_str_radix(hex, 16).ok().filter(|code| *code > 0);
    }
    if token.chars().all(|c| c.is_ascii_digit()) {
        if let Ok(code) = token.parse::<u16>() {
            if code > 0 {
                return Some(code);
            }
        }
    }
    if token.len() == 1 {
        let c = token.as_bytes()[0];
        if c.is_ascii_lowercase() {
            return Some(30 + (c - b'a') as u16);
        }
        if c.is_ascii_digit() {
            return Some(match c {
                b'1' => 2,
                b'2' => 3,
                b'3' => 4,
                b'4' => 5,
                b'5' => 6,
                b'6' => 7,
                b'7' => 8,
                b'8' => 9,
                b'9' => 10,
                _ => 11,
            });
        }
        return match token.as_str() {
            "-" => Some(12),
            "=" => Some(13),
            "," => Some(51),
            "." => Some(52),
            "/" => Some(53),
            ";" => Some(39),
            "'" => Some(40),
            "\\" => Some(43),
            "`" => Some(41),
            "[" => Some(26),
            "]" => Some(27),
            _ => None,
        };
    }

    Some(match token.as_str() {
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
    })
}

fn parse_chord(chord: &str) -> Result<Vec<u16>, String> {
    let trimmed = chord.trim();
    if trimmed.is_empty() {
        return Err("key chord is empty".into());
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
                return Err("key chord has more than one non-modifier key".into());
            }
            main = Some(code);
        } else {
            return Err(format!("unknown key '{token}'"));
        }
    }
    let main = main.ok_or_else(|| "key chord has no non-modifier key".to_string())?;
    modifiers.push(main);
    Ok(modifiers)
}

#[repr(C)]
#[derive(Clone, Copy)]
struct InputEvent {
    time: libc::timeval,
    type_: u16,
    code: u16,
    value: i32,
}

struct UInputClient {
    fd: RawFd,
}

impl UInputClient {
    fn connect() -> Result<Self, String> {
        let path = ydotool_socket_path();
        let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
        addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
        let bytes = path.as_bytes();
        if bytes.len() >= addr.sun_path.len() {
            return Err(format!("ydotool socket path is too long: {path}"));
        }
        for (index, byte) in bytes.iter().enumerate() {
            addr.sun_path[index] = *byte as libc::c_char;
        }
        let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_DGRAM, 0) };
        if fd < 0 {
            return Err(format!("socket: {}", io::Error::last_os_error()));
        }
        let result = unsafe {
            libc::connect(
                fd,
                &addr as *const _ as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t,
            )
        };
        if result != 0 {
            let err = io::Error::last_os_error();
            unsafe {
                libc::close(fd);
            }
            return Err(format!("connect {path}: {err}"));
        }
        Ok(Self { fd })
    }

    fn emit(&self, type_: u16, code: u16, value: i32) -> Result<(), String> {
        let event = InputEvent {
            time: libc::timeval {
                tv_sec: 0,
                tv_usec: 0,
            },
            type_,
            code,
            value,
        };
        let sent = unsafe {
            libc::send(
                self.fd,
                &event as *const _ as *const libc::c_void,
                std::mem::size_of::<InputEvent>(),
                0,
            )
        };
        if sent < 0 {
            return Err(format!("ydotool send: {}", io::Error::last_os_error()));
        }
        Ok(())
    }

    fn syn(&self) -> Result<(), String> {
        self.emit(EV_SYN, SYN_REPORT, 0)
    }

    fn key(&self, code: u16, pressed: bool) -> Result<(), String> {
        self.emit(EV_KEY, code, if pressed { 1 } else { 0 })?;
        self.syn()
    }

    fn wheel(&self, steps: i32) -> Result<(), String> {
        self.emit(EV_REL, REL_WHEEL, steps)?;
        self.syn()
    }
}

impl Drop for UInputClient {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.fd);
        }
    }
}

fn ydotool_socket_path() -> String {
    if let Ok(path) = env::var("YDOTOOL_SOCKET") {
        if !path.is_empty() {
            return path;
        }
    }
    if let Ok(runtime) = env::var("XDG_RUNTIME_DIR") {
        if !runtime.is_empty() {
            return format!("{runtime}/.ydotool_socket");
        }
    }
    "/tmp/.ydotool_socket".to_string()
}

#[derive(Serialize)]
struct Status {
    version: String,
    pid: u32,
    ready: bool,
    running: bool,
    mode: String,
    name: String,
    action: String,
    cps: f64,
    effective_cps: f64,
    hold_ms: f64,
    effective_hold_ms: f64,
    jitter_pct: f64,
    clicks: u64,
    keys: u64,
    macro_step: u32,
    macro_step_count: u32,
    macro_repeat: u32,
    macro_repeat_index: u32,
    started_at_ms: u64,
    updated_at_ms: u64,
    reason: String,
    error: String,
}

impl Status {
    fn new() -> Self {
        Self {
            version: VERSION.to_string(),
            pid: std::process::id(),
            ready: false,
            running: false,
            mode: "repeat".into(),
            name: String::new(),
            action: "left".into(),
            cps: default_cps(),
            effective_cps: 0.0,
            hold_ms: default_hold(),
            effective_hold_ms: 0.0,
            jitter_pct: 0.0,
            clicks: 0,
            keys: 0,
            macro_step: 0,
            macro_step_count: 0,
            macro_repeat: 0,
            macro_repeat_index: 0,
            started_at_ms: 0,
            updated_at_ms: 0,
            reason: String::new(),
            error: String::new(),
        }
    }
}

struct Engine {
    client: Option<UInputClient>,
    config_path: PathBuf,
    status_path: PathBuf,
    heartbeat_path: Option<PathBuf>,
    heartbeat_started_ms: u64,
    last_status_write_ms: u64,
    config: Config,
    status: Status,
    active_keys: Vec<u16>,
    active_buttons: Vec<u16>,
    rng: Rng,
}

impl Engine {
    fn new(config_path: PathBuf, status_path: PathBuf, heartbeat_path: Option<PathBuf>) -> Self {
        let mut engine = Self {
            client: None,
            config_path,
            status_path,
            heartbeat_path,
            heartbeat_started_ms: now_ms(),
            last_status_write_ms: 0,
            config: Config::default(),
            status: Status::new(),
            active_keys: Vec::new(),
            active_buttons: Vec::new(),
            rng: Rng::new(),
        };
        engine.status.error = String::new();
        engine
    }

    fn write_status(&mut self, force: bool) {
        let now = now_ms();
        if !force && now.saturating_sub(self.last_status_write_ms) < 200 {
            return;
        }
        self.last_status_write_ms = now;
        self.status.updated_at_ms = now;
        self.status.pid = std::process::id();
        let Some(parent) = self.status_path.parent() else {
            return;
        };
        if fs::create_dir_all(parent).is_err() {
            return;
        }
        let tmp = self.status_path.with_extension("json.tmp");
        let Ok(data) = serde_json::to_vec_pretty(&self.status) else {
            return;
        };
        if fs::write(&tmp, data).is_ok() {
            let _ = fs::rename(&tmp, &self.status_path);
        }
    }

    fn load_config(&mut self) -> Result<(), String> {
        let raw = fs::read_to_string(&self.config_path)
            .map_err(|err| format!("read {}: {err}", self.config_path.display()))?;
        let parsed: Config =
            serde_json::from_str(&raw).map_err(|err| format!("parse config: {err}"))?;
        let config = parsed.normalized()?;
        self.config = config;
        self.status.error.clear();
        self.status.mode = match self.config.mode {
            Mode::Repeat => "repeat".into(),
            Mode::Macro => "macro".into(),
        };
        self.status.name = self
            .config
            .macro_def
            .as_ref()
            .map(|def| def.name.clone())
            .unwrap_or_default();
        self.status.cps = self.config.cps;
        self.status.hold_ms = self.config.hold_ms;
        self.status.jitter_pct = self.config.jitter_pct;
        self.status.action = action_label(&self.config.action);
        Ok(())
    }

    fn check_control(&mut self) -> Option<String> {
        if take(&SIG_TERM) || take(&SIG_INT) {
            return Some("shutdown".into());
        }
        if take(&SIG_USR2) {
            return Some("stopped".into());
        }
        if take(&SIG_HUP) {
            if let Err(err) = self.load_config() {
                self.status.error = err;
            }
        }
        None
    }

    fn heartbeat_ok(&mut self) -> bool {
        let Some(path) = &self.heartbeat_path else {
            return true;
        };
        match fs::metadata(path).and_then(|meta| meta.modified()) {
            Ok(modified) => match SystemTime::now().duration_since(modified) {
                Ok(age) => age < Duration::from_secs(3),
                Err(_) => true,
            },
            Err(_) => now_ms().saturating_sub(self.heartbeat_started_ms) < 3000,
        }
    }

    fn ensure_client(&mut self) -> Result<(), String> {
        if self.client.is_none() {
            self.client = Some(UInputClient::connect()?);
        }
        Ok(())
    }

    fn key(&mut self, code: u16, pressed: bool) -> Result<(), String> {
        self.ensure_client()?;
        self.client
            .as_ref()
            .ok_or_else(|| "ydotool client is unavailable".to_string())?
            .key(code, pressed)
    }

    fn wheel(&mut self, steps: i32) -> Result<(), String> {
        self.ensure_client()?;
        self.client
            .as_ref()
            .ok_or_else(|| "ydotool client is unavailable".to_string())?
            .wheel(steps)
    }

    fn press_action(&mut self, action: &Action) -> Result<(), String> {
        match action {
            Action::Mouse { button } => {
                let code = mouse_code(button)?;
                if !self.active_buttons.contains(&code) {
                    self.key(code, true)?;
                    self.active_buttons.push(code);
                }
            }
            Action::Key { chord } => {
                let codes = parse_chord(chord)?;
                for code in codes {
                    if !self.active_keys.contains(&code) {
                        self.key(code, true)?;
                        self.active_keys.push(code);
                    }
                }
            }
        }
        Ok(())
    }

    fn release_action(&mut self, action: &Action) -> Result<(), String> {
        match action {
            Action::Mouse { button } => {
                let code = mouse_code(button)?;
                if let Some(index) = self.active_buttons.iter().position(|item| *item == code) {
                    self.active_buttons.remove(index);
                    self.key(code, false)?;
                }
            }
            Action::Key { chord } => {
                let codes = parse_chord(chord)?;
                for code in codes.into_iter().rev() {
                    if let Some(index) = self.active_keys.iter().position(|item| *item == code) {
                        self.active_keys.remove(index);
                        self.key(code, false)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn release_all(&mut self) {
        while let Some(code) = self.active_keys.pop() {
            let _ = self.key(code, false);
        }
        while let Some(code) = self.active_buttons.pop() {
            let _ = self.key(code, false);
        }
    }

    fn stop(&mut self, reason: &str) {
        self.release_all();
        self.status.running = false;
        self.status.reason = reason.to_string();
        self.status.macro_step = 0;
        self.status.macro_step_count = 0;
        self.status.macro_repeat_index = 0;
        self.write_status(true);
    }

    fn sleep_checked(&mut self, ms: f64) -> Option<String> {
        if ms <= 0.0 {
            return self.check_control();
        }
        let deadline = now_ns() + (ms * 1_000_000.0) as u64;
        loop {
            let now = now_ns();
            if now >= deadline {
                return self.check_control();
            }
            sleep_until(deadline);
            if let Some(reason) = self.check_control() {
                return Some(reason);
            }
            if !self.heartbeat_ok() {
                return Some("heartbeat".into());
            }
        }
    }

    fn run_repeat(&mut self) -> String {
        let started = now_ns();
        let mut next = started;
        loop {
            if let Some(reason) = self.check_control() {
                return reason;
            }
            if !self.heartbeat_ok() {
                return "heartbeat".into();
            }

            let cps = self.config.cps.clamp(1.0, 1000.0);
            let period_ns = 1_000_000_000.0 / cps;
            let max_hold_ns = (period_ns * 0.8).max(1_000_000.0);
            let hold_ns = ((self.config.hold_ms * 1_000_000.0).max(0.0)).min(max_hold_ns);
            let jitter = self.config.jitter_pct / 100.0;

            self.status.effective_cps = 1_000_000_000.0 / period_ns;
            self.status.effective_hold_ms = hold_ns / 1_000_000.0;
            self.status.cps = cps;
            self.status.hold_ms = self.config.hold_ms;
            self.status.macro_step_count = 0;

            let fire_at = next.max(now_ns());
            sleep_until(fire_at);
            if let Some(reason) = self.check_control() {
                return reason;
            }

            let action = self.config.action.clone();
            if let Err(err) = self.press_action(&action) {
                self.status.error = err;
                return "error".into();
            }

            let release_at = fire_at + hold_ns as u64;
            loop {
                let now = now_ns();
                if now >= release_at {
                    break;
                }
                sleep_until(release_at);
                if let Some(reason) = self.check_control() {
                    let _ = self.release_action(&action);
                    return reason;
                }
                if !self.heartbeat_ok() {
                    let _ = self.release_action(&action);
                    return "heartbeat".into();
                }
            }
            if let Err(err) = self.release_action(&action) {
                self.status.error = err;
                return "error".into();
            }

            self.status.clicks = self.status.clicks.saturating_add(1);
            if matches!(action, Action::Key { .. }) {
                self.status.keys = self.status.keys.saturating_add(1);
            }
            self.write_status(false);

            let max_runtime = self.config.max_runtime_s;
            if max_runtime > 0.0
                && (now_ns().saturating_sub(started) as f64) >= max_runtime * 1_000_000_000.0
            {
                return "duration".into();
            }

            let factor = if jitter > 0.0 {
                1.0 + (self.rng.next_f64() * 2.0 - 1.0) * jitter
            } else {
                1.0
            };
            let period = (period_ns * factor).max(hold_ns + 500_000.0);
            next = fire_at.saturating_add(period as u64);
            if next <= now_ns() {
                next = now_ns() + 1_000_000;
            }
        }
    }

    fn run_macro(&mut self) -> String {
        let started = now_ns();
        let mut repeat_index: u32 = 0;
        loop {
            if let Some(reason) = self.check_control() {
                return reason;
            }
            if !self.heartbeat_ok() {
                return "heartbeat".into();
            }
            let Some(def) = self.config.macro_def.clone() else {
                self.status.error = "no macro configured".into();
                return "error".into();
            };
            if def.steps.is_empty() {
                self.status.error = "macro has no steps".into();
                return "error".into();
            }

            self.status.name = def.name.clone();
            self.status.macro_step_count = def.steps.len() as u32;
            self.status.macro_repeat = def.repeats;
            self.status.macro_repeat_index = repeat_index.saturating_add(1);

            for (index, step) in def.steps.iter().enumerate() {
                self.status.macro_step = index as u32 + 1;
                self.status.macro_repeat_index = repeat_index.saturating_add(1);
                if let Some(reason) = self.check_control() {
                    return reason;
                }
                let result = match step {
                    Step::Key {
                        chord,
                        hold_ms,
                        delay_ms,
                    } => {
                        let codes = match parse_chord(chord) {
                            Ok(codes) => codes,
                            Err(err) => {
                                self.status.error = err;
                                return "error".into();
                            }
                        };
                        for code in &codes {
                            if !self.active_keys.contains(code) {
                                if let Err(err) = self.key(*code, true) {
                                    self.status.error = err;
                                    return "error".into();
                                }
                                self.active_keys.push(*code);
                            }
                        }
                        let hold = hold_ms.clamp(0.0, 2000.0);
                        if let Some(reason) = self.sleep_checked(hold) {
                            for code in codes.iter().rev() {
                                if let Some(pos) = self.active_keys.iter().position(|c| c == code) {
                                    self.active_keys.remove(pos);
                                    let _ = self.key(*code, false);
                                }
                            }
                            return reason;
                        }
                        for code in codes.iter().rev() {
                            if let Some(pos) = self.active_keys.iter().position(|c| c == code) {
                                self.active_keys.remove(pos);
                                if let Err(err) = self.key(*code, false) {
                                    self.status.error = err;
                                    return "error".into();
                                }
                            }
                        }
                        self.status.clicks = self.status.clicks.saturating_add(1);
                        self.status.keys = self.status.keys.saturating_add(1);
                        self.sleep_checked(delay_ms.clamp(0.0, 60000.0))
                    }
                    Step::Mouse {
                        button,
                        hold_ms,
                        delay_ms,
                    } => {
                        let code = match mouse_code(button) {
                            Ok(code) => code,
                            Err(err) => {
                                self.status.error = err;
                                return "error".into();
                            }
                        };
                        if !self.active_buttons.contains(&code) {
                            if let Err(err) = self.key(code, true) {
                                self.status.error = err;
                                return "error".into();
                            }
                            self.active_buttons.push(code);
                        }
                        let hold = hold_ms.clamp(0.0, 2000.0);
                        if let Some(reason) = self.sleep_checked(hold) {
                            if let Some(pos) = self.active_buttons.iter().position(|c| *c == code) {
                                self.active_buttons.remove(pos);
                                let _ = self.key(code, false);
                            }
                            return reason;
                        }
                        if let Some(pos) = self.active_buttons.iter().position(|c| *c == code) {
                            self.active_buttons.remove(pos);
                            if let Err(err) = self.key(code, false) {
                                self.status.error = err;
                                return "error".into();
                            }
                        }
                        self.status.clicks = self.status.clicks.saturating_add(1);
                        self.sleep_checked(delay_ms.clamp(0.0, 60000.0))
                    }
                    Step::Delay { ms } => self.sleep_checked(ms.clamp(0.0, 60000.0)),
                    Step::Scroll { direction, steps } => {
                        let value = if direction.eq_ignore_ascii_case("up") {
                            *steps
                        } else {
                            -*steps
                        };
                        if let Err(err) = self.wheel(value) {
                            self.status.error = err;
                            return "error".into();
                        }
                        self.write_status(false);
                        None
                    }
                };
                if let Some(reason) = result {
                    return reason;
                }
                self.write_status(false);

                let max_runtime = if self.config.max_runtime_s > 0.0 {
                    self.config.max_runtime_s
                } else if def.max_runtime_s > 0.0 {
                    def.max_runtime_s
                } else if def.repeats == 0 {
                    3600.0
                } else {
                    0.0
                };
                if max_runtime > 0.0
                    && (now_ns().saturating_sub(started) as f64) >= max_runtime * 1_000_000_000.0
                {
                    return "duration".into();
                }
            }

            repeat_index = repeat_index.saturating_add(1);
            if def.repeats != 0 && repeat_index >= def.repeats {
                return "completed".into();
            }
            if let Some(reason) = self.sleep_checked(def.loop_delay_ms.clamp(0.0, 60000.0)) {
                return reason;
            }
        }
    }

    fn run_daemon(&mut self) {
        install_signals();
        unsafe {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
        }
        if let Err(err) = self.load_config() {
            self.status.error = err;
        }
        self.status.ready = true;
        self.write_status(true);

        loop {
            if take(&SIG_TERM) || take(&SIG_INT) {
                self.stop("shutdown");
                break;
            }
            if take(&SIG_USR2) {
                self.stop("stopped");
            }
            if take(&SIG_HUP) {
                if let Err(err) = self.load_config() {
                    self.status.error = err;
                    self.write_status(true);
                }
            }
            if take(&SIG_USR1) {
                if let Err(err) = self.load_config() {
                    self.status.error = err;
                    self.write_status(true);
                } else if let Err(err) = self.ensure_client() {
                    self.status.error = err;
                    self.write_status(true);
                } else {
                    self.status.running = true;
                    self.status.reason.clear();
                    self.status.error.clear();
                    self.status.clicks = 0;
                    self.status.keys = 0;
                    self.status.started_at_ms = now_ms();
                    self.status.macro_step = 0;
                    self.status.macro_repeat_index = 0;
                    self.write_status(true);
                    let reason = match self.config.mode {
                        Mode::Repeat => self.run_repeat(),
                        Mode::Macro => self.run_macro(),
                    };
                    self.stop(&reason);
                    if reason == "shutdown" || reason == "heartbeat" {
                        break;
                    }
                }
            }
            if !self.heartbeat_ok() {
                self.stop("heartbeat");
                break;
            }
            self.write_status(false);
            sleep_until(now_ns() + 100_000_000);
        }
    }
}

fn action_label(action: &Action) -> String {
    match action {
        Action::Mouse { button } => button.clone(),
        Action::Key { chord } => format!("key:{chord}"),
    }
}

fn print_help() {
    println!(
        "clickerd {VERSION}
Input sequencer for the Noctalia Clicker plugin.

Usage:
  clickerd --daemon --config PATH --status PATH [--heartbeat PATH]
  clickerd --dry-run --config PATH
  clickerd --click-once BUTTON
  clickerd --release-all
  clickerd --version

Signals:
  SIGHUP   reload config
  SIGUSR1  start
  SIGUSR2  stop
  SIGTERM  stop and exit"
    );
}

fn arg_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|index| args.get(index + 1))
        .cloned()
}

fn dry_run(config_path: &Path) -> Result<(), String> {
    let raw = fs::read_to_string(config_path)
        .map_err(|err| format!("read {}: {err}", config_path.display()))?;
    let parsed: Config =
        serde_json::from_str(&raw).map_err(|err| format!("parse config: {err}"))?;
    let config = parsed.normalized()?;
    println!("{}", serde_json::to_string_pretty(&config).unwrap());

    match config.mode {
        Mode::Repeat => {
            let cps = config.cps.clamp(1.0, 1000.0);
            let period = 1000.0 / cps;
            let max_hold = period * 0.8;
            let hold = config.hold_ms.min(max_hold);
            println!(
                "repeat: {:.3} CPS, hold {:.3} ms, jitter {:.1}%",
                cps, hold, config.jitter_pct
            );
            for index in 0..10 {
                println!(
                    "  t={:8.1} ms  down {}  up +{:.1} ms",
                    index as f64 * period,
                    action_label(&config.action),
                    hold
                );
            }
        }
        Mode::Macro => {
            let def = config.macro_def.as_ref().unwrap();
            println!(
                "macro '{}': {} steps, repeat {}",
                def.name,
                def.steps.len(),
                if def.repeats == 0 {
                    "loop".to_string()
                } else {
                    def.repeats.to_string()
                }
            );
            let mut t = 0.0;
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
                    Step::Scroll { direction, steps } => {
                        (format!("scroll {direction} x{steps}"), 0.0, 0.0)
                    }
                };
                println!(
                    "  {:2}. t={:8.1} ms  {}  hold {:.1} ms  delay {:.1} ms",
                    index + 1,
                    t,
                    label,
                    hold,
                    delay
                );
                t += hold + delay;
            }
            println!("  loop delay {:.1} ms", def.loop_delay_ms);
        }
    }
    Ok(())
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_help();
        return;
    }
    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        println!("clickerd {VERSION}");
        return;
    }

    if args.iter().any(|arg| arg == "--release-all") {
        match UInputClient::connect() {
            Ok(client) => {
                for code in [272u16, 273, 274, 275, 276, 277, 278, 279] {
                    let _ = client.key(code, false);
                }
                for code in [29u16, 42, 56, 125, 57] {
                    let _ = client.key(code, false);
                }
                println!("released");
            }
            Err(err) => {
                eprintln!("{err}");
                std::process::exit(1);
            }
        }
        return;
    }

    if let Some(button) = arg_value(&args, "--click-once") {
        match UInputClient::connect() {
            Ok(client) => {
                let code = match mouse_code(&button) {
                    Ok(code) => code,
                    Err(err) => {
                        eprintln!("{err}");
                        std::process::exit(1);
                    }
                };
                if client.key(code, true).is_ok() {
                    sleep_ms(8.0);
                    let _ = client.key(code, false);
                }
                println!("clicked {button}");
            }
            Err(err) => {
                eprintln!("{err}");
                std::process::exit(1);
            }
        }
        return;
    }

    let Some(config_path) = arg_value(&args, "--config") else {
        eprintln!("clickerd: --config is required");
        std::process::exit(2);
    };
    let config_path = PathBuf::from(config_path);

    if args.iter().any(|arg| arg == "--dry-run") {
        if let Err(err) = dry_run(&config_path) {
            eprintln!("{err}");
            std::process::exit(1);
        }
        return;
    }

    let Some(status_path) = arg_value(&args, "--status") else {
        eprintln!("clickerd: --status is required with --daemon");
        std::process::exit(2);
    };
    let heartbeat_path = arg_value(&args, "--heartbeat").map(PathBuf::from);
    let mut engine = Engine::new(config_path, PathBuf::from(status_path), heartbeat_path);
    engine.run_daemon();
}

