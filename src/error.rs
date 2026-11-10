//! Diagnostic logging and error rendering for applications.

use wllib::error::WireError;
use wllib::fmt_lite::StringOnStack;

use crate::io::write_stderr;

pub fn write_wire_diagnostic(prefix: &str, err: &WireError) {
  let mut s = StringOnStack::<128>::new();
  s.push(prefix);
  match err {
    WireError::Sys(e) => {
      s.push(" syscall failed: ")
        .push(e.call)
        .push(" (")
        .push(e.errno)
        .push(")");
    }
    WireError::Protocol(p) => {
      s.push(" wayland protocol error: object ")
        .push(p.object_id)
        .push(", code ")
        .push(p.code)
        .push(", message: ")
        .push(p.message.as_str());
    }
    WireError::InvalidPath => {
      s.push(" WAYLAND_DISPLAY or XDG_RUNTIME_DIR not set or socket path invalid");
    }
    WireError::BufferTooSmall => {
      s.push(" receive buffer too small for wayland framing");
    }
    WireError::ConnectionClosed => {
      s.push(" connection closed by compositor");
    }
  }
  s.push("\n");
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
