//! Audio capture and the Terraria bite-sound template.
//!
//! Terraria's fishing bobber plays a dedicated splash when a fish bites. It is
//! `Content/Sounds/Splash_5.xnb` in the local installation, usually with a
//! randomized pitch. Detection is therefore a matched filter over a set of
//! pitch-shifted copies of that sample, not computer vision.
//!
//! The waveform is read from the local game installation at runtime, so no
//! copyrighted audio is distributed with the plugin.

use std::collections::VecDeque;
use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use crate::config::FisherConfig;
use crate::error::AudioError;

/// Capture rate. Everything downstream assumes this.
pub const SAMPLE_RATE: u32 = 48_000;

/// Longest correlation template. 120 ms is enough of the attack to be unique
/// while keeping detection latency (and CPU) low.
pub const TEMPLATE_SECONDS: f32 = 0.12;

/// How much source audio is kept so pitch-shifted variants (which lengthen or
/// shorten the sample) can still fill a full template window.
pub const TEMPLATE_SOURCE_SECONDS: f32 = 0.3;

/// Three seconds of history is plenty for a detector that only looks at the
/// last few hundred milliseconds.
const MAX_CAPTURE_SAMPLES: usize = SAMPLE_RATE as usize * 3;

/// A running `parec` process feeding a ring buffer.
#[derive(Debug)]
pub struct Capture {
    child: Child,
    shared: Arc<Mutex<Buffer>>,
    reader: Option<JoinHandle<()>>,
}

#[derive(Debug, Default)]
struct Buffer {
    samples: VecDeque<f32>,
    total: u64,
    error: Option<String>,
    closed: bool,
}

impl Buffer {
    fn push(&mut self, sample: f32) {
        self.samples.push_back(sample);
        self.total += 1;
    }
}

/// The newest audio, enough for the detector's window.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub samples: Vec<f32>,
    pub total: u64,
    /// `true` once the capture process stopped producing audio.
    pub closed: bool,
    pub error: Option<String>,
}

impl Capture {
    /// Starts capturing. An empty device means "the default speaker monitor".
    pub fn start(device: &str) -> Result<Self, AudioError> {
        let device = if device.trim().is_empty() {
            default_monitor()?
        } else {
            device.trim().to_string()
        };

        let mut child = Command::new("parec")
            .args([
                "--format=s16le",
                "--rate=48000",
                "--channels=1",
                "--raw",
                "--latency-msec=30",
                "--client-name=clickerd",
            ])
            .arg(format!("--device={device}"))
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|source| AudioError::Capture(format!("cannot start parec: {source}")))?;

        let mut stdout = child.stdout.take().ok_or_else(|| {
            AudioError::Capture("parec started without a capture pipe".to_string())
        })?;

        let shared = Arc::new(Mutex::new(Buffer::default()));
        let sink = Arc::clone(&shared);
        let reader = thread::spawn(move || {
            let mut bytes = [0u8; 8192];
            // A pipe read can split a 16-bit sample. Keep the stranded byte
            // for the next read instead of dropping it, which would shift every
            // following sample by one byte.
            let mut carry: Option<u8> = None;
            loop {
                match stdout.read(&mut bytes) {
                    Ok(0) => break,
                    Ok(count) => {
                        let mut rest = &bytes[..count];
                        let mut buffer = sink.lock().unwrap_or_else(|error| error.into_inner());
                        if let Some(first) = carry.take() {
                            if let Some((&second, tail)) = rest.split_first() {
                                buffer.push(i16::from_le_bytes([first, second]) as f32 / 32768.0);
                                rest = tail;
                            } else {
                                carry = Some(first);
                                continue;
                            }
                        }
                        let (pairs, remainder) = rest.as_chunks::<2>();
                        for pair in pairs {
                            buffer.push(i16::from_le_bytes(*pair) as f32 / 32768.0);
                        }
                        carry = remainder.first().copied();
                        while buffer.samples.len() > MAX_CAPTURE_SAMPLES {
                            buffer.samples.pop_front();
                        }
                    }
                    Err(source) => {
                        let mut buffer = sink.lock().unwrap_or_else(|error| error.into_inner());
                        buffer.error = Some(format!("audio capture failed: {source}"));
                        break;
                    }
                }
            }
            let mut buffer = sink.lock().unwrap_or_else(|error| error.into_inner());
            buffer.closed = true;
        });

        Ok(Self {
            child,
            shared,
            reader: Some(reader),
        })
    }

    /// Copies the newest `tail` samples for the detector.
    pub fn snapshot(&self, tail: usize) -> Snapshot {
        let buffer = self
            .shared
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let count = tail.min(buffer.samples.len());
        let samples = buffer
            .samples
            .iter()
            .skip(buffer.samples.len() - count)
            .copied()
            .collect();
        Snapshot {
            samples,
            total: buffer.total,
            closed: buffer.closed,
            error: buffer.error.clone(),
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

/// The monitor source of the default sink, so we hear what the game hears.
fn default_monitor() -> Result<String, AudioError> {
    let output = Command::new("pactl")
        .args(["get-default-sink"])
        .output()
        .map_err(|source| AudioError::Capture(format!("cannot run pactl: {source}")))?;
    let sink = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if sink.is_empty() {
        return Err(AudioError::Capture(
            "pactl did not report a default sink".to_string(),
        ));
    }
    Ok(format!("{sink}.monitor"))
}

/// A correlation template plus where it came from (for the panel).
#[derive(Debug, Clone)]
pub struct Template {
    pub samples: Vec<f32>,
    pub source: String,
    pub sample_rate: u32,
}

/// Loads the bite template: an explicit path, then the game installation.
pub fn load_template(config: &FisherConfig) -> Result<Template, AudioError> {
    if let Some(template) = load_optional(&config.template)? {
        return Ok(template);
    }
    if let Some(path) = find_game_sound() {
        let (samples, rate) = read_sound_file(&path)?;
        return prepare(samples, rate, path.display().to_string());
    }
    Err(AudioError::TemplateMissing)
}

fn load_optional(path: &str) -> Result<Option<Template>, AudioError> {
    let path = path.trim();
    if path.is_empty() {
        return Ok(None);
    }
    let path = expand_home(path);
    if !path.exists() {
        return Ok(None);
    }
    let (samples, rate) = read_sound_file(&path)?;
    Ok(Some(prepare(samples, rate, path.display().to_string())?))
}

fn expand_home(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

/// Reads either a `.xnb` sound asset or a raw little-endian `f32` template.
fn read_sound_file(path: &Path) -> Result<(Vec<f32>, u32), AudioError> {
    let data = fs::read(path).map_err(|source| AudioError::TemplateRead {
        path: path.to_path_buf(),
        source,
    })?;

    let is_xnb = path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("xnb"));
    if is_xnb {
        return parse_xnb(&data, path);
    }

    if data.len() % 4 != 0 || data.len() < 4 {
        return Err(AudioError::TemplateParse {
            path: path.to_path_buf(),
            message: "raw templates must be little-endian f32 samples".to_string(),
        });
    }
    let (chunks, _) = data.as_chunks::<4>();
    let samples = chunks
        .iter()
        .map(|chunk| f32::from_le_bytes(*chunk))
        .collect();
    Ok((samples, SAMPLE_RATE))
}

/// Trims to the attack and resamples to the capture rate.
fn prepare(samples: Vec<f32>, source_rate: u32, source: String) -> Result<Template, AudioError> {
    if samples.len() < 64 || source_rate == 0 {
        return Err(AudioError::TemplateParse {
            path: PathBuf::from(&source),
            message: "template is too short".to_string(),
        });
    }

    let peak = samples
        .iter()
        .fold(0.0_f32, |peak, value| peak.max(value.abs()));
    if peak <= f32::EPSILON {
        return Err(AudioError::TemplateParse {
            path: PathBuf::from(&source),
            message: "template is silent".to_string(),
        });
    }

    // Start a millisecond before the first audible sample so the attack is not
    // clipped, then keep only the onset.
    let onset = samples
        .iter()
        .position(|value| value.abs() > peak * 0.05)
        .unwrap_or(0)
        .saturating_sub((source_rate / 1_000) as usize);
    let window = (source_rate as f32 * TEMPLATE_SOURCE_SECONDS) as usize;
    let end = (onset + window).min(samples.len());
    let trimmed = &samples[onset..end];

    let length = (SAMPLE_RATE as f32 * TEMPLATE_SOURCE_SECONDS) as usize;
    let resampled = resample(trimmed, source_rate, SAMPLE_RATE, length);

    Ok(Template {
        samples: resampled,
        source,
        sample_rate: SAMPLE_RATE,
    })
}

fn resample(samples: &[f32], source_rate: u32, target_rate: u32, length: usize) -> Vec<f32> {
    let step = source_rate as f64 / target_rate as f64;
    let mut out = Vec::with_capacity(length);
    for index in 0..length {
        let position = index as f64 * step;
        let base = position.floor() as usize;
        let fraction = (position - base as f64) as f32;
        let first = samples.get(base).copied().unwrap_or(0.0);
        let second = samples.get(base + 1).copied().unwrap_or(first);
        out.push(first + (second - first) * fraction);
    }
    out
}

/// Parses the subset of XNB used by Terraria's sound assets.
fn parse_xnb(data: &[u8], path: &Path) -> Result<(Vec<f32>, u32), AudioError> {
    let fail = |message: &str| AudioError::TemplateParse {
        path: path.to_path_buf(),
        message: message.to_string(),
    };

    if data.len() < 14 || &data[..3] != b"XNB" {
        return Err(fail("not an XNB file"));
    }

    let mut cursor = 10usize; // magic, platform, version, flags, file size
    let readers = read_7bit(data, &mut cursor).ok_or_else(|| fail("truncated header"))?;
    for _ in 0..readers {
        let length = read_7bit(data, &mut cursor).ok_or_else(|| fail("truncated reader"))?;
        cursor = cursor
            .checked_add(length)
            .filter(|end| *end <= data.len())
            .ok_or_else(|| fail("truncated reader name"))?;
        cursor = cursor
            .checked_add(4)
            .filter(|end| *end <= data.len())
            .ok_or_else(|| fail("truncated reader version"))?;
    }
    read_7bit(data, &mut cursor).ok_or_else(|| fail("truncated object table"))?;

    // Terraria's writer leaves a byte between the object table and the
    // WAVEFORMATEX block, and the exact reader version has changed over time.
    // Probing a few offsets is more robust than pinning one layout, and every
    // candidate is still fully bounds-checked.
    let object_start = cursor;
    let mut last_error = "no audio format block found".to_string();
    for skip in 0..=8 {
        match parse_sound_object(data, object_start + skip) {
            Ok(sound) => return Ok(sound),
            Err(message) => last_error = message,
        }
    }
    Err(fail(&last_error))
}

/// Parses `[format length][WAVEFORMATEX][data length][PCM]` at `at`.
fn parse_sound_object(data: &[u8], at: usize) -> Result<(Vec<f32>, u32), String> {
    let mut cursor = at;
    let format_length =
        read_i32(data, &mut cursor).ok_or_else(|| "truncated format size".to_string())?;
    if !(16..=64).contains(&format_length) {
        return Err("unexpected audio format block".to_string());
    }
    let format_length = format_length as usize;
    let format = data
        .get(cursor..cursor + format_length)
        .ok_or_else(|| "truncated format block".to_string())?;
    cursor += format_length;

    let tag = u16::from_le_bytes([format[0], format[1]]);
    let channels = u16::from_le_bytes([format[2], format[3]]) as usize;
    let sample_rate = u32::from_le_bytes([format[4], format[5], format[6], format[7]]);
    let bits = u16::from_le_bytes([format[14], format[15]]);
    if tag != 1 {
        return Err("only uncompressed PCM sound assets are supported".to_string());
    }
    if bits != 16 {
        return Err("only 16-bit sound assets are supported".to_string());
    }
    if channels == 0 || channels > 8 {
        return Err("unsupported channel count".to_string());
    }
    if !(8_000..=192_000).contains(&sample_rate) {
        return Err("unsupported sample rate".to_string());
    }

    let data_length =
        read_i32(data, &mut cursor).ok_or_else(|| "truncated data size".to_string())?;
    if data_length <= 0 {
        return Err("empty audio data".to_string());
    }
    let data_length = data_length as usize;
    let pcm = data
        .get(cursor..cursor + data_length)
        .ok_or_else(|| "truncated audio data".to_string())?;
    if pcm.len() % (2 * channels) != 0 {
        return Err("truncated audio data".to_string());
    }

    let (pairs, _) = pcm.as_chunks::<2>();
    let samples = pairs
        .iter()
        .map(|[low, high]| i16::from_le_bytes([*low, *high]) as f32 / 32768.0)
        .collect::<Vec<f32>>()
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect();

    Ok((samples, sample_rate))
}

fn read_7bit(data: &[u8], cursor: &mut usize) -> Option<usize> {
    let mut value = 0usize;
    let mut shift = 0;
    loop {
        let byte = *data.get(*cursor)?;
        *cursor += 1;
        value |= ((byte & 0x7f) as usize) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
        shift += 7;
        if shift > 28 {
            return None;
        }
    }
}

fn read_i32(data: &[u8], cursor: &mut usize) -> Option<i32> {
    let bytes = data.get(*cursor..*cursor + 4)?;
    *cursor += 4;
    Some(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

/// Looks for the game's bite sound in the usual Steam library locations.
///
/// `Splash_5` is the sample the fishing bobber actually plays when a fish
/// bites; `fish_splash` is kept as a fallback for other builds.
pub fn find_game_sound() -> Option<PathBuf> {
    const RELATIVE: [&str; 2] = [
        "steamapps/common/Terraria/Content/Sounds/Splash_5.xnb",
        "steamapps/common/Terraria/Content/Sounds/Custom/fish_splash.xnb",
    ];
    let mut libraries = Vec::new();

    if let Some(home) = env::var_os("HOME") {
        let home = PathBuf::from(home);
        libraries.push(home.join(".steam/steam"));
        libraries.push(home.join(".local/share/Steam"));
        libraries.push(home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"));
        libraries.push(home.join(".steam/root"));
    }
    if let Some(extra) = env::var_os("STEAM_LIBRARY_PATH") {
        libraries.extend(env::split_paths(&extra));
    }

    let mut searched = Vec::new();
    for library in &libraries {
        let candidates = [library.clone(), library.join("steamapps")];
        for candidate in candidates {
            if searched.contains(&candidate) {
                continue;
            }
            for relative in RELATIVE {
                let sound = candidate.join(relative);
                if sound.exists() {
                    return Some(sound);
                }
            }
            for path in read_library_folders(&candidate.join("steamapps/libraryfolders.vdf")) {
                for relative in RELATIVE {
                    let sound = path.join(relative);
                    if sound.exists() {
                        return Some(sound);
                    }
                }
            }
            searched.push(candidate);
        }
    }
    None
}

/// Pulls `"path" "..."` entries out of Steam's library index.
fn read_library_folders(index: &Path) -> Vec<PathBuf> {
    let Ok(text) = fs::read_to_string(index) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            let rest = line.strip_prefix("\"path\"")?;
            let start = rest.find('"')? + 1;
            let tail = &rest[start..];
            let end = tail.find('"')?;
            Some(PathBuf::from(&tail[..end]))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampling_preserves_shape() {
        let source: Vec<f32> = (0..1000)
            .map(|index| (index as f32 / 100.0).sin())
            .collect();
        let out = resample(&source, 44_100, SAMPLE_RATE, 2000);
        assert_eq!(out.len(), 2000);
        assert!(out.iter().all(|value| value.abs() <= 1.0));
    }

    #[test]
    fn prepare_trims_to_the_onset() {
        let mut samples = vec![0.0; 4410];
        samples.extend((0..44100).map(|index| ((index as f32) * 0.1).sin()));
        let template = prepare(samples, 44_100, "test".into()).unwrap();
        assert_eq!(
            template.samples.len(),
            (SAMPLE_RATE as f32 * TEMPLATE_SOURCE_SECONDS) as usize
        );
        assert!(template.samples.iter().any(|value| value.abs() > 0.1));
    }

    #[test]
    fn seven_bit_integers_match_the_xnb_encoding() {
        let mut cursor = 0;
        assert_eq!(read_7bit(&[0x7f], &mut cursor), Some(127));
        let mut cursor = 0;
        assert_eq!(read_7bit(&[0x80, 0x01], &mut cursor), Some(128));
        let mut cursor = 0;
        assert_eq!(read_7bit(&[0x81, 0x7f], &mut cursor), Some(16_257));
    }
}
