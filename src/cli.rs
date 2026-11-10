use core::ffi::{CStr, c_char};
use rustix::fs::{Access, access};
use wllib::fmt_lite::StringOnStack;

use crate::io::{write_stderr, write_stdout};

pub struct Config {
  pub image_path: StringOnStack<256>,
  pub namespace: StringOnStack<256>,
  pub blur: u32,
}
impl Default for Config {
  fn default() -> Self {
    Self {
      image_path: StringOnStack::new(),
      namespace: *StringOnStack::new().push_str("wallpaper"),
      blur: 0,
    }
  }
}

const HELP: &str = concat!(
  "Static wallpapers for Wayland\n\n",
  "Usage: rustbg [OPTIONS] IMAGE\n\n",
  "Arguments:\n",
  "  IMAGE                 Wallpaper image\n\n",
  "Options:\n",
  "  -n, --namespace NAME  Layer name [default: wallpaper]\n",
  "      --blur SIGMA      Blur strength, 0-1024 [default: 0]\n",
  "  -h, --help            Show help\n",
);

fn invalid(message: &str) -> i32 {
  write_stderr("[rustbg] ");
  write_stderr(message);
  write_stderr("\nUsage: rustbg [OPTIONS] IMAGE\nTry 'rustbg --help' for more information.\n");
  1
}

/// # Safety
/// `argv` must contain `argc` valid, null-terminated C strings.
pub unsafe fn parse(argc: isize, argv: *const *mut c_char) -> Result<Config, i32> {
  let mut config = Config::default();
  if argc < 1 || argv.is_null() {
    return Err(1);
  }
  let args = unsafe { core::slice::from_raw_parts(argv, argc as usize) };
  let mut args = args[1..]
    .iter()
    .map(|&arg| unsafe { CStr::from_ptr(arg) }.to_str());
  let mut positional = false;
  while let Some(arg) = args.next() {
    let arg = arg.map_err(|_| invalid("arguments must be UTF-8"))?;
    if !positional && arg == "--" {
      positional = true;
    } else if !positional && (arg == "--help" || arg.starts_with("-h")) {
      write_stdout(HELP);
      return Err(0);
    } else if !positional && arg.starts_with('-') && arg != "-" {
      let (option, attached) = if let Some(long) = arg.strip_prefix("--") {
        let (key, value) = long
          .split_once('=')
          .map_or((long, None), |(k, v)| (k, Some(v)));
        (key, value)
      } else {
        if let Some(value) = arg.strip_prefix("-n") {
          (
            "namespace",
            if value.is_empty() { None } else { Some(value) },
          )
        } else {
          return Err(invalid("unknown option"));
        }
      };
      if option != "namespace" && option != "blur" {
        return Err(invalid("unknown option"));
      }
      let value = match attached {
        Some(value) => value,
        None => args
          .next()
          .and_then(Result::ok)
          .filter(|value| !value.starts_with('-'))
          .ok_or_else(|| invalid("missing option value"))?,
      };
      if option == "blur" {
        config.blur = value
          .parse()
          .ok()
          .filter(|&n| n <= 1024)
          .ok_or_else(|| invalid("blur must be an integer between 0 and 1024"))?;
      } else {
        // The layer-shell request must fit wllib's 256-byte message.
        if value.is_empty() || value.len() > 227 {
          return Err(invalid("namespace must contain 1-227 bytes"));
        }
        config.namespace.clear();
        config.namespace.push(value);
      }
    } else {
      if !config.image_path.is_empty() || arg.is_empty() || arg.len() > config.image_path.capacity()
      {
        return Err(invalid("expected one image path of at most 255 bytes"));
      }
      config.image_path.push(arg);
    }
  }
  if config.image_path.is_empty() {
    return Err(invalid("no image path supplied"));
  }
  access(config.image_path.as_c_str(), Access::EXISTS)
    .map_err(|_| invalid("cannot open image file"))?;
  Ok(config)
}
