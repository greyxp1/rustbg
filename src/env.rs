use core::ffi::{CStr, c_char};
use core::ptr;
use core::sync::atomic::{AtomicPtr, Ordering};

use wllib::error::WireError;
use wllib::fmt_lite::StringOnStack;
use wllib::transport::Connection;

static ENVP: AtomicPtr<*const c_char> = AtomicPtr::new(ptr::null_mut());

/// # Safety
/// Initialize once with the process's environment, which must remain valid and immutable.
pub unsafe fn set_envp(envp: *const *const c_char) {
  ENVP.store(envp.cast_mut(), Ordering::Release);
}

pub fn get_envp() -> *const *const c_char {
  ENVP.load(Ordering::Acquire).cast_const()
}

pub fn get_env(key: &str) -> Option<&'static str> {
  let mut cur = get_envp();
  while !cur.is_null() && !unsafe { *cur }.is_null() {
    // The inherited environment lives for the process lifetime and is never modified.
    let entry = unsafe { CStr::from_ptr(*cur) }.to_bytes();
    if let Some(value) = entry
      .strip_prefix(key.as_bytes())
      .and_then(|s| s.strip_prefix(b"="))
    {
      return core::str::from_utf8(value).ok();
    }
    cur = unsafe { cur.add(1) };
  }
  None
}

pub fn connect_wayland() -> Result<Connection, WireError> {
  let display = get_env("WAYLAND_DISPLAY").unwrap_or("wayland-0");
  if display.starts_with('/') {
    return Connection::connect_to(display);
  }
  let runtime = get_env("XDG_RUNTIME_DIR")
    .ok_or(WireError::InvalidPath)?
    .trim_end_matches('/');
  let mut path = StringOnStack::<128>::new();
  if runtime.len() + 1 + display.len() > path.capacity() {
    return Err(WireError::InvalidPath);
  }
  path.push(runtime).push("/").push(display);
  Connection::connect_to(path.as_str())
}
