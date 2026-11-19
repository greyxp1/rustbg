#![no_std]
#![no_main]

pub mod config;
pub mod error;
pub mod state;

use core::ffi::{c_char, c_int};
use core::ptr;
use core::sync::atomic::{AtomicI32, Ordering};

use rustix::event::{PollFd, PollFlags, poll};
use rustix::fd::{AsRawFd, BorrowedFd, OwnedFd};
use rustix::fs::{Access, access};
use rustix::io::{Errno, read, write};
use rustix::pipe::{PipeFlags, pipe_with};
use shared::cli::Arg::Value;
use shared::cli::{ArgParser, get_args_iter};
use shared::exit::die;
use shared::io::write_stderr;
use shared::signal::{KernelSigaction, sigaction, signal};
use wllib::dispatch::dispatch_pending;
use wllib::error::WireError::ConnectionClosed;
use wllib::error::{SysError, SysResultExt};
use wllib::registry::crawl;

use crate::config::Config;
use crate::error::AppError;
use crate::state::State;

const SIGCHLD: c_int = 17;
const SIGUSR1: c_int = 10;
const SIG_IGN: usize = 1;

// write end of the self-pipe the SIGUSR1 handler writes to.
static WAKE_FD: AtomicI32 = AtomicI32::new(-1);

extern "C" fn handle_sigusr1(_sig: c_int) {
  let fd = WAKE_FD.load(Ordering::Relaxed);
  if fd >= 0 {
    let _ = write(unsafe { BorrowedFd::borrow_raw(fd) }, &[1]);
  }
}

fn write_usage() -> c_int {
  write_stderr("Usage: rustidle <config-file>\n");
  1
}

fn drain_wakeups(wake_rx: &OwnedFd) -> usize {
  let mut total = 0usize;
  let mut buf = [0u8; 64];
  while let Ok(n) = read(wake_rx, &mut buf) {
    if n == 0 {
      break;
    }
    total = total.saturating_add(n);
  }
  total
}

fn run(config_path: &str) -> Result<(), AppError> {
  // tells kernel to reap child processes automatically, avoiding waitpid
  let _ = unsafe { signal(SIGCHLD, SIG_IGN).syserr("sigaction") };

  // self-pipe
  let (wake_rx, wake_tx) = pipe_with(PipeFlags::CLOEXEC | PipeFlags::NONBLOCK).syserr("pipe2")?;
  WAKE_FD.store(wake_tx.as_raw_fd(), Ordering::Relaxed);
  unsafe {
    let act = KernelSigaction::new(handle_sigusr1 as *const () as usize, 0);
    if sigaction(SIGUSR1, &act, ptr::null_mut()).is_err() {
      die("[rustidle] failed to set action handler");
    }
  }

  let mut conn = shared::connect_wayland()?;
  let config = Config::load(config_path)?;
  let mut state = State::init(config);
  crawl(&mut conn, &mut state)?;

  state.create_notifications(&mut conn)?;
  let mut suspended = false;

  loop {
    // handle everything already buffered before sleeping
    while dispatch_pending(&mut conn, &mut state)? > 0 {}

    let mut fds = [
      PollFd::new(&conn, PollFlags::IN),
      PollFd::new(&wake_rx, PollFlags::IN),
    ];
    match poll(&mut fds, None) {
      Ok(_) => {}
      Err(Errno::INTR) => continue,
      Err(e) => return Err(SysError::from_errno("poll", e).into()),
    }
    let wayland_ready = !fds[0].revents().is_empty();
    let wake_ready = !fds[1].revents().is_empty();

    // each SIGUSR1 toggles, so an even number of signals since the last wakeup is a no-op
    if wake_ready && drain_wakeups(&wake_rx) % 2 != 0 {
      suspended.toggle();
      if suspended {
        state.destroy_notifications(&mut conn)?;
      } else {
        state.create_notifications(&mut conn)?;
      }
    }

    if wayland_ready {
      match conn.fill() {
        Ok(()) => {}
        Err(ConnectionClosed) => break,
        Err(e) => return Err(e.into()),
      }
    }
  }
  Ok(())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn main(argc: isize, argv: *const *mut c_char) -> c_int {
  if argc != 2 {
    return write_usage();
  }
  let args_iter = unsafe { get_args_iter(argc as usize, argv) };
  let mut parser = ArgParser::new(args_iter);
  let mut config_path = "";

  while let Some(arg) = parser.next_arg() {
    let Value(path) = arg else {
      return write_usage();
    };
    config_path = path;
  }

  if access(config_path, Access::EXISTS).is_err() {
    write_stderr("[rustidle] failed to open config file.\n");
    return 1;
  }

  if let Err(e) = run(config_path) {
    e.write_diagnostic();
    return 1;
  }
  0
}
