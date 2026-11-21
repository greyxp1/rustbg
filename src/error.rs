//! Diagnostic logging and error rendering for applications.

use wllib::error::{SysError, WireError};

use crate::fmt_lite::StringOnStack;
use crate::io::write_stderr;

/// Write a wire error to stderr tagged with `prefix`.
pub fn write_wire_diagnostic(prefix: &str, err: &WireError) {
  match err {
    WireError::Sys(e) => write_sys_diagnostic(prefix, e),
    WireError::Protocol(p) => {
      let mut s = StringOnStack::<128>::new();
      s.push(prefix)
        .push(" wayland protocol error: object ")
        .push(p.object_id)
        .push(", code ")
        .push(p.code)
        .push(", message: ")
        .push(p.message())
        .push("\n");
      write_stderr(s);
    }
    WireError::InvalidPath => {
      write_stderr(prefix);
      write_stderr(" WAYLAND_DISPLAY or XDG_RUNTIME_DIR not set or socket path invalid\n");
    }
    WireError::BufferTooSmall => {
      write_stderr(prefix);
      write_stderr(" receive buffer too small for wayland framing\n");
    }
    WireError::ConnectionClosed => {
      write_stderr(prefix);
      write_stderr(" connection closed by compositor\n");
    }
    WireError::Parse(err) => {
      write_stderr(prefix);
      write_stderr(" wayland message parse error: ");
      match err {
        wllib::wire::MessageError::BadSize => write_stderr("bad message size\n"),
        wllib::wire::MessageError::Truncated => write_stderr("truncated message\n"),
        wllib::wire::MessageError::BadString => write_stderr("malformed string argument\n"),
        wllib::wire::MessageError::TrailingData => {
          write_stderr("unconsumed trailing message bytes\n")
        }
        wllib::wire::MessageError::Overflow => write_stderr("message writer buffer overflow\n"),
      };
    }
    WireError::MessageTooLarge => {
      write_stderr(prefix);
      write_stderr(" message exceeds receive buffer capacity\n");
    }
    WireError::TooManyFds => {
      write_stderr(prefix);
      write_stderr(" too many file descriptors passed\n");
    }
    WireError::FdOverflow => {
      write_stderr(prefix);
      write_stderr(" file descriptor queue overflow\n");
    }
    WireError::IdsExhausted => {
      write_stderr(prefix);
      write_stderr(" client object IDs exhausted\n");
    }
  }
}

/// Write a syscall error to stderr tagged with `prefix`.
pub fn write_sys_diagnostic(prefix: &str, err: &SysError) {
  let mut s = StringOnStack::<128>::new();
  s.push(prefix)
    .push(" syscall failed: ")
    .push(err.call)
    .push(" (")
    .push(err.errno)
    .push(")\n");
  write_stderr(s);
}

// Application-level errors.
//
// Wraps [`wllib::WireError`] for anything related to the wayland connection/protocol. Each
// variant carries enough context to render a real diagnostic via `write_diagnostic`.
pub enum AppError {
  // A failure from the wayland transport/protocol layer.
  Wire(WireError),
  // FFmpeg produced fewer bytes than the expected `width * height * 4`.
  ImageDecodeError,
}

impl AppError {
  pub fn write_diagnostic(&self) {
    match self {
      AppError::Wire(e) => write_wire_diagnostic("[rustbg]", e),
      AppError::ImageDecodeError => {
        write_stderr("[rustbg] image decode error: FFmpeg produced fewer bytes than expected\n")
      }
    }
  }
}

impl From<WireError> for AppError {
  fn from(e: WireError) -> Self {
    AppError::Wire(e)
  }
}
