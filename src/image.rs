use core::ffi::c_char;
use core::ptr;
use rustix::fd::OwnedFd;
use rustix::fs::{MemfdFlags, memfd_create};

use crate::process::{Fork, exec_ffmpeg, exit, fork, reset_sigpipe};
use rustix::fs::{Mode, OFlags, open};
use rustix::io::Errno;
use rustix::pipe::{PipeFlags, SpliceFlags, pipe_with, splice};
use rustix::process::{Pid, WaitOptions, waitpid};
use rustix::stdio;
use wllib::error::SysResultExt;
use wllib::fmt_lite::StringOnStack;

use crate::cli::Config;
use crate::error::AppError;

fn exec_into_fd<const N: usize>(
  argv: [*const c_char; N],
  fd: &OwnedFd,
  size: usize,
) -> Result<(), AppError> {
  let (read_fd, write_fd) = pipe_with(PipeFlags::CLOEXEC).syserr("pipe")?;

  let fork_res = fork().syserr("fork")?;
  match fork_res {
    Fork::Child => {
      drop(read_fd);

      if stdio::dup2_stdout(&write_fd).is_err() || reset_sigpipe().is_err() {
        exit(1);
      }

      drop(write_fd);

      if let Ok(devnull) = open(c"/dev/null", OFlags::WRONLY, Mode::empty()) {
        let _ = stdio::dup2_stderr(&devnull);
        drop(devnull);
      }

      unsafe { exec_ffmpeg(argv.as_ptr()) };
    }
    Fork::Parent(pid) => {
      drop(write_fd);

      let mut offset = 0usize;
      let result = loop {
        if offset == size {
          break Ok(());
        }
        match splice(
          &read_fd,
          None,
          fd,
          None,
          size - offset,
          SpliceFlags::empty(),
        ) {
          Ok(0) => break Ok(()),
          Ok(count) => offset += count,
          Err(Errno::INTR) => continue,
          Err(error) => break Err(error),
        }
      };

      // Always close the pipe and reap the child, including after a transfer error.
      drop(read_fd);
      loop {
        match waitpid(Pid::from_raw(pid as i32), WaitOptions::empty()) {
          Err(Errno::INTR) => continue,
          status => {
            status.syserr("waitpid")?;
            break;
          }
        }
      }
      result.syserr("splice")?;

      if offset != size {
        return Err(AppError::ImageDecodeError);
      }
    }
  }
  Ok(())
}

fn load_and_scale(
  out_width: u32,
  out_height: u32,
  fd: &OwnedFd,
  size: usize,
  config: &Config,
) -> Result<(), AppError> {
  let mut filter = StringOnStack::<256>::new();

  // Retain the source pixels covering the target before scaling, rounding outward.
  filter
    .push("crop='min(iw,ceil(ih*")
    .push(out_width)
    .push("/")
    .push(out_height)
    .push("))':'min(ih,ceil(iw*")
    .push(out_height)
    .push("/")
    .push(out_width)
    .push("))':exact=1,scale=")
    .push(out_width)
    .push(":")
    .push(out_height)
    // Preserve the aspect ratio when the integer source crop differs from the target.
    .push(":force_original_aspect_ratio=increase,crop=")
    .push(out_width)
    .push(":")
    .push(out_height);

  if config.blur != 0 {
    filter.push(",gblur=sigma=").push(config.blur);
  }

  let argv: [*const c_char; 13] = [
    c"ffmpeg".as_ptr(),
    c"-filter_threads".as_ptr(),
    c"1".as_ptr(),
    c"-i".as_ptr(),
    config.image_path.as_ptr(),
    c"-vf".as_ptr(),
    filter.as_ptr(),
    c"-f".as_ptr(),
    c"rawvideo".as_ptr(),
    c"-pix_fmt".as_ptr(),
    c"bgra".as_ptr(),
    c"-".as_ptr(),
    ptr::null(),
  ];
  exec_into_fd(argv, fd, size)
}

pub fn get_image_fd(out_width: u32, out_height: u32, config: &Config) -> Result<OwnedFd, AppError> {
  let stride = out_width * 4; // 4 bytes per pixel (BGRA)
  let size = (stride * out_height) as usize;

  let fd = memfd_create(
    c"rustbg-wayland-shm",
    MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING | MemfdFlags::NOEXEC_SEAL,
  )
  .syserr("memfd_create")?;

  load_and_scale(out_width, out_height, &fd, size, config)?;

  Ok(fd)
}
