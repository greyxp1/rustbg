//! Environment Variable Parsing

use core::ffi::{CStr, c_char};
use core::sync::atomic::{AtomicPtr, Ordering};
use core::{ptr, str};

use wllib::error::WireError;
use wllib::fmt_lite::StringOnStack;
use wllib::transport::Connection;

static ENVP: AtomicPtr<*const c_char> = AtomicPtr::new(ptr::null_mut());

/// Sets the `envp` pointer.
pub fn set_envp(envp: *const *const c_char) {
  ENVP.store(envp.cast_mut(), Ordering::Release);
}

/// Returns the raw `envp` pointer, if set.
pub fn get_envp() -> *const *const c_char {
  ENVP.load(Ordering::Acquire).cast_const()
}

/// Fetch an environment variable into a `StringOnStack`.
pub fn get_env<const N: usize>(key: &str, out: &mut StringOnStack<N>) -> bool {
  scan_envp(get_envp(), key, out)
}

fn scan_envp<const N: usize>(
  mut cur: *const *const c_char,
  key: &str,
  out: &mut StringOnStack<N>,
) -> bool {
  let key_bytes = key.as_bytes();
  while !cur.is_null() && !unsafe { *cur }.is_null() {
    let cstr = unsafe { CStr::from_ptr(*cur) };
    let bytes = cstr.to_bytes();
    if bytes.starts_with(key_bytes)
      && bytes.get(key_bytes.len()) == Some(&b'=')
      && let Ok(val) = str::from_utf8(&bytes[key_bytes.len() + 1..])
    {
      out.clear();
      out.push(val);
      return true;
    }
    cur = unsafe { cur.add(1) };
  }
  false
}

/// Resolves the Wayland socket path by checking `WAYLAND_DISPLAY` and `XDG_RUNTIME_DIR`.
pub fn resolve_wayland_socket<const N: usize>(path: &mut StringOnStack<N>) -> bool {
  let mut display = StringOnStack::<64>::new();
  let mut runtime = StringOnStack::<64>::new();

  let found_display = get_env("WAYLAND_DISPLAY", &mut display);
  let found_runtime = get_env("XDG_RUNTIME_DIR", &mut runtime);
  let display_str = if found_display {
    display.as_str()
  } else {
    "wayland-0"
  };

  path.clear();
  if display_str.starts_with('/') {
    path.push(display_str);
  } else {
    if !found_runtime {
      return false;
    }
    path.push(runtime.as_str());
    if !path.ends_with('/') {
      path.push("/");
    }
    path.push(display_str);
  }
  true
}

/// Resolves the Wayland socket path and connects to it.
pub fn connect_wayland() -> Result<Connection, WireError> {
  let mut path = StringOnStack::<128>::new();
  if !resolve_wayland_socket(&mut path) {
    return Err(WireError::InvalidPath);
  }
  Connection::connect_to(path.as_str())
}
