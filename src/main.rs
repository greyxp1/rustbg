#![no_std]
#![no_main]

mod cli;
mod entry;
mod env;
mod error;
mod image;
mod io;
mod process;
mod wayland;

use core::ffi::{c_char, c_int};

use crate::error::write_wire_diagnostic;
use wllib::dispatch::dispatch_once;
use wllib::error::WireError::ConnectionClosed;
use wllib::registry::crawl;

use crate::wayland::State;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn main(argc: isize, argv: *const *mut c_char) -> c_int {
  let config = match unsafe { cli::parse(argc, argv) } {
    Ok(config) => config,
    Err(status) => return status,
  };

  let mut conn = match crate::env::connect_wayland() {
    Ok(c) => c,
    Err(e) => {
      write_wire_diagnostic("[rustbg]", &e);
      return 1;
    }
  };

  let mut state = State::init(config);

  if let Err(e) = crawl(&mut conn, &mut state) {
    write_wire_diagnostic("[rustbg]", &e);
    return 1;
  }

  state.create_surfaces(&mut conn);

  loop {
    match dispatch_once(&mut conn, &mut state) {
      Ok(()) => (),
      Err(ConnectionClosed) => break,
      Err(e) => {
        write_wire_diagnostic("[rustbg]", &e);
        break;
      }
    }
  }
  0
}
