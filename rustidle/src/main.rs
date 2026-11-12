#![no_std]
#![no_main]

pub mod config;
pub mod error;
pub mod state;

use core::ffi::{c_char, c_int};
use core::ptr;
use core::sync::atomic::{AtomicBool, Ordering};

use rustix::fs::{Access, access};
use shared::cli::Arg::Value;
use shared::cli::{ArgParser, get_args_iter};
use shared::exit::die;
use shared::io::write_stderr;
use shared::signal::{KernelSigaction, sigaction, signal};
use wllib::dispatch::dispatch_once;
use wllib::error::SysResultExt;
use wllib::error::WireError::ConnectionClosed;
use wllib::registry::crawl;

use crate::config::Config;
use crate::error::AppError;
use crate::state::State;

const SIGCHLD: c_int = 17;
const SIGUSR1: c_int = 10;
const SIG_IGN: usize = 1;

// flag to track the suspended state
static IS_SUSPENDED: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_sigusr1(_sig: c_int) {
  // fetch_xor with true inverts the current boolean value
  IS_SUSPENDED.fetch_xor(true, Ordering::Relaxed);
}

fn write_usage() -> c_int {
  write_stderr("Usage: rustidle <config-file>\n");
  1
}

fn run(config_path: &str) -> Result<(), AppError> {
  // tells kernel to reap child processes automatically, avoiding waitpid
  let _ = unsafe { signal(SIGCHLD, SIG_IGN).syserr("sigaction") };
  // handle SIGUSR1
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
  let mut was_suspended = false;

  // main wayland event dispatch loop
  loop {
    if IS_SUSPENDED.load(Ordering::Relaxed) {
      state.destroy_notifications(&mut conn)?;
      was_suspended = true;
    }

    while IS_SUSPENDED.load(Ordering::Relaxed) {
      rustix::event::pause();
    }

    if was_suspended {
      state.create_notifications(&mut conn)?;
      was_suspended = false;
    }

    match dispatch_once(&mut conn, &mut state) {
      Ok(()) => (),
      Err(ConnectionClosed) => break,
      Err(e) => {
        if IS_SUSPENDED.load(Ordering::Relaxed) {
          continue;
        }
        return Err(e.into());
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
