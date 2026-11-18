//! Signal handling.
//!
//! The handler only sets atomic flags; the daemon translates them into actions
//! on its next tick. `SA_RESTART` is intentionally not requested, so a signal
//! also interrupts `clock_nanosleep` and stops a run promptly.

use std::io;
use std::sync::atomic::{AtomicU32, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    Terminate,
    Interrupt,
    Stop,
    Reload,
    Start,
}

const TERMINATE: u32 = 1 << 0;
const INTERRUPT: u32 = 1 << 1;
const STOP: u32 = 1 << 2;
const RELOAD: u32 = 1 << 3;
const START: u32 = 1 << 4;

static PENDING: AtomicU32 = AtomicU32::new(0);

impl Signal {
    const fn flag(self) -> u32 {
        match self {
            Self::Terminate => TERMINATE,
            Self::Interrupt => INTERRUPT,
            Self::Stop => STOP,
            Self::Reload => RELOAD,
            Self::Start => START,
        }
    }

    fn from_raw(signal: libc::c_int) -> Option<Self> {
        match signal {
            libc::SIGTERM => Some(Self::Terminate),
            libc::SIGINT => Some(Self::Interrupt),
            libc::SIGUSR2 => Some(Self::Stop),
            libc::SIGHUP => Some(Self::Reload),
            libc::SIGUSR1 => Some(Self::Start),
            _ => None,
        }
    }

    fn take(self) -> bool {
        PENDING.fetch_and(!self.flag(), Ordering::SeqCst) & self.flag() != 0
    }
}

extern "C" fn handle(signal: libc::c_int) {
    if let Some(signal) = Signal::from_raw(signal) {
        PENDING.fetch_or(signal.flag(), Ordering::SeqCst);
    }
}

/// Installs handlers for every signal the helper understands.
pub fn install() -> io::Result<()> {
    for signal in [
        libc::SIGHUP,
        libc::SIGUSR1,
        libc::SIGUSR2,
        libc::SIGTERM,
        libc::SIGINT,
    ] {
        // SAFETY: `action` is fully initialized and the handler only touches
        // atomics, which is async-signal-safe.
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = handle as *const () as usize;
            action.sa_flags = 0;
            libc::sigemptyset(&mut action.sa_mask);
            if libc::sigaction(signal, &action, std::ptr::null_mut()) != 0 {
                return Err(io::Error::last_os_error());
            }
        }
    }
    Ok(())
}

/// Takes the highest-priority pending signal, including [`Signal::Start`].
pub fn next() -> Option<Signal> {
    const PRIORITY: [Signal; 5] = [
        Signal::Terminate,
        Signal::Interrupt,
        Signal::Stop,
        Signal::Reload,
        Signal::Start,
    ];
    PRIORITY.into_iter().find(|signal| signal.take())
}

/// Takes the next signal that interrupts a run, leaving [`Signal::Start`] queued
/// for the daemon loop.
pub fn take_control() -> Option<Signal> {
    const PRIORITY: [Signal; 4] = [
        Signal::Terminate,
        Signal::Interrupt,
        Signal::Stop,
        Signal::Reload,
    ];
    PRIORITY.into_iter().find(|signal| signal.take())
}

#[cfg(test)]
mod tests {
    use super::*;

    // One test: the queue is process-wide, so parallel tests would race.
    #[test]
    fn queues_and_prioritizes_signals() {
        PENDING.store(START, Ordering::SeqCst);
        assert_eq!(take_control(), None);
        assert_eq!(next(), Some(Signal::Start));
        assert_eq!(next(), None);

        PENDING.store(STOP | RELOAD, Ordering::SeqCst);
        assert_eq!(take_control(), Some(Signal::Stop));
        assert_eq!(take_control(), Some(Signal::Reload));
        assert_eq!(take_control(), None);
    }
}
