//! File-descriptor write helpers.

use rustix::fd::AsFd;
use rustix::io::{Errno, write};
use rustix::stdio;
use wllib::error::SysError;

/// Writes `msg` to the file descriptor. Retries on partial write and `EINTR` until all bytes have
/// been written or a real error occurs.
fn write_fd<Fd: AsFd>(fd: Fd, msg: impl AsRef<[u8]>) -> Result<(), SysError> {
  let mut msg = msg.as_ref();
  while !msg.is_empty() {
    let res = match write(fd.as_fd(), msg) {
      Ok(0) => break,
      Ok(n) => n,
      Err(Errno::INTR) => continue,
      Err(err) => return Err(SysError::from_errno("write", err)),
    };
    msg = &msg[res..];
  }
  Ok(())
}

/// Writes the buffer to stderr.
pub fn write_stderr(msg: impl AsRef<[u8]>) {
  let fd = unsafe { stdio::stderr() };
  let _ = write_fd(fd, msg);
}
