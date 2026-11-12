#![no_std]
#![no_main]

pub mod error;
pub mod image_load;
pub mod shm;
pub mod state;

use core::ffi::{c_char, c_int};

use rustix::fs::{Access, access};
use shared::cli::{Arg, ArgParser, get_args_iter};
use shared::io::write_stderr;

use crate::error::AppError;
use crate::state::{Config, State};

fn write_usage() -> c_int {
  write_stderr("Usage: rustbg [-f | --fill] [-n | --namespace <name>] <image path>\n");
  1
}

fn run(config: Config) -> Result<(), AppError> {
  shared::daemon_runner(State::init(config), |conn, state| state.setup_outputs(conn))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn main(argc: isize, argv: *const *mut c_char) -> c_int {
  let mut config = Config::default();

  if argc <= 0 || argv.is_null() {
    return 1;
  }

  let args_iter = unsafe { get_args_iter(argc as usize, argv) };

  let mut parser = ArgParser::new(args_iter);
  while let Some(arg) = parser.next_arg() {
    match arg {
      Arg::Short('f') | Arg::Long("fill") => config.fill = true,
      Arg::Short('n') | Arg::Long("namespace") => {
        let Some(namespace) = parser.next_val() else {
          write_stderr("[rustbg] no namespace provided.\n");
          return write_usage();
        };
        config.namespace.clear();
        config.namespace.push(namespace);
      }
      Arg::Value(path) => {
        config.image_path.push(path);
      }
      Arg::Short(_) | Arg::Long(_) => return write_usage(),
    }
  }

  if config.image_path.is_empty() {
    write_stderr("[rustbg] no image path supplied\n");
    return write_usage();
  }

  if access(config.image_path.as_c_str(), Access::EXISTS).is_err() {
    write_stderr("[rustbg] cannot open image file.\n");
    return write_usage();
  }

  if let Err(e) = run(config) {
    e.write_diagnostic();
    return 1;
  }

  0
}
