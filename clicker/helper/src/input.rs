//! Protocol client for `ydotoold`.
//!
//! ydotoold accepts datagrams containing raw `struct input_event` values on an
//! `AF_UNIX` `SOCK_DGRAM` socket. `std::os::unix::net::UnixDatagram` gives us
//! that without hand-rolling `sockaddr_un` handling.

use std::env;
use std::os::unix::net::UnixDatagram;
use std::path::PathBuf;
use std::slice;

use crate::error::InputError;
use crate::keys::KeyCode;

const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const EV_REL: u16 = 2;
const SYN_REPORT: u16 = 0;
const REL_WHEEL: u16 = 8;

/// A connected datagram socket to the running ydotoold.
#[derive(Debug)]
pub struct YdotoolClient {
    socket: UnixDatagram,
}

impl YdotoolClient {
    pub fn connect() -> Result<Self, InputError> {
        let path = socket_path();
        let socket = UnixDatagram::unbound().map_err(|source| InputError::Connect {
            path: path.clone(),
            source,
        })?;
        socket
            .connect(&path)
            .map_err(|source| InputError::Connect { path, source })?;
        Ok(Self { socket })
    }

    /// Presses or releases one key or button, followed by a `SYN_REPORT`.
    pub fn key(&self, code: KeyCode, pressed: bool) -> Result<(), InputError> {
        self.emit(EV_KEY, code.as_u16(), i32::from(pressed))?;
        self.sync()
    }

    /// Scrolls `steps` notches; negative values scroll down.
    pub fn wheel(&self, steps: i32) -> Result<(), InputError> {
        self.emit(EV_REL, REL_WHEEL, steps)?;
        self.sync()
    }

    fn sync(&self) -> Result<(), InputError> {
        self.emit(EV_SYN, SYN_REPORT, 0)
    }

    fn emit(&self, kind: u16, code: u16, value: i32) -> Result<(), InputError> {
        let event = InputEvent::new(kind, code, value);
        self.socket
            .send(event.as_bytes())
            .map_err(InputError::Send)?;
        Ok(())
    }
}

#[repr(C)]
struct InputEvent {
    time: libc::timeval,
    kind: u16,
    code: u16,
    value: i32,
}

impl InputEvent {
    fn new(kind: u16, code: u16, value: i32) -> Self {
        Self {
            time: libc::timeval {
                tv_sec: 0,
                tv_usec: 0,
            },
            kind,
            code,
            value,
        }
    }

    fn as_bytes(&self) -> &[u8] {
        // SAFETY: `InputEvent` is `#[repr(C)]`, contains only plain integers
        // (`libc::timeval` is two `c_long`s), and is fully initialized. Viewing
        // it as bytes for the lifetime of `&self` cannot observe padding that
        // is not already part of the value, and `send` only reads it.
        unsafe {
            slice::from_raw_parts(
                self as *const Self as *const u8,
                std::mem::size_of::<Self>(),
            )
        }
    }
}

fn socket_path() -> PathBuf {
    if let Some(path) = env::var_os("YDOTOOL_SOCKET").filter(|value| !value.is_empty()) {
        return PathBuf::from(path);
    }
    if let Some(runtime) = env::var_os("XDG_RUNTIME_DIR").filter(|value| !value.is_empty()) {
        return PathBuf::from(runtime).join(".ydotool_socket");
    }
    PathBuf::from("/tmp/.ydotool_socket")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_event_matches_the_kernel_layout() {
        // struct input_event on 64-bit Linux: timeval (16) + type (2) +
        // code (2) + value (4).
        assert_eq!(std::mem::size_of::<InputEvent>(), 24);
        assert_eq!(std::mem::align_of::<InputEvent>(), 8);
    }

    #[test]
    fn event_bytes_are_the_struct_bytes() {
        let event = InputEvent::new(EV_KEY, 273, 1);
        let bytes = event.as_bytes();
        assert_eq!(bytes.len(), 24);
        assert_eq!(u16::from_ne_bytes([bytes[16], bytes[17]]), EV_KEY);
        assert_eq!(u16::from_ne_bytes([bytes[18], bytes[19]]), 273);
        assert_eq!(
            i32::from_ne_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]),
            1
        );
    }
}
