//! Typed errors for the helper.
//!
//! Each layer has its own error type, and [`Error`] only composes them. That
//! keeps messages precise without stringly typed plumbing between modules.

use std::io;
use std::path::PathBuf;

/// A configuration file that could not be read, parsed, or accepted.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("cannot parse {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("{message}")]
    Invalid { message: String },

    #[error("step {index}: {message}")]
    InvalidStep { index: usize, message: String },
}

impl ConfigError {
    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid {
            message: message.into(),
        }
    }

    pub(crate) fn step(index: usize, message: impl Into<String>) -> Self {
        Self::InvalidStep {
            index,
            message: message.into(),
        }
    }
}

/// A failure while talking to `ydotoold`.
#[derive(Debug, thiserror::Error)]
pub enum InputError {
    #[error("cannot connect to ydotoold at {path}: {source}")]
    Connect {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("cannot send input event to ydotoold: {0}")]
    Send(#[source] io::Error),
}

/// A failure while capturing audio or preparing the bite template.
#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("audio capture is unavailable: {0}")]
    Capture(String),

    #[error("cannot read the bite template at {path}: {source}")]
    TemplateRead {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("cannot use the bite template at {path}: {message}")]
    TemplateParse { path: PathBuf, message: String },

    #[error(
        "no bite template found: point the template setting at Splash_5.xnb \
         or at a raw f32 template"
    )]
    TemplateMissing,
}

/// A command line that cannot be turned into a [`crate::cli::Command`].
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum CliError {
    #[error("{0} needs a value")]
    MissingValue(&'static str),

    #[error("{0} is required")]
    MissingOption(&'static str),

    #[error("no mode selected; pass --daemon or --dry-run")]
    MissingMode,

    #[error("unknown argument '{0}'")]
    UnknownArgument(String),

    #[error("unknown mouse button '{0}'")]
    UnknownButton(String),

    #[error("{0} does not accept '{1}'")]
    UnknownValue(&'static str, String),
}

/// Top-level error, spanning configuration, input, and startup.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Config(#[from] ConfigError),

    #[error(transparent)]
    Input(#[from] InputError),

    #[error(transparent)]
    Audio(#[from] AudioError),

    #[error("cannot serialize config: {0}")]
    Serialize(#[from] serde_json::Error),

    #[error("system error: {0}")]
    System(#[from] io::Error),
}
