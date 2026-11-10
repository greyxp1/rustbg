use core::ffi::c_char;
use core::ptr;
use core::slice;
use rustix::fd::OwnedFd;
use rustix::fs::{MemfdFlags, ftruncate, memfd_create};
use rustix::mm::{MapFlags, ProtFlags, mmap, munmap};

use crate::process::{Fork, exec_ffmpeg, exit, fork, reset_sigpipe};
use rustix::fs::{Mode, OFlags, open};
use rustix::io::{Errno, read};
use rustix::pipe::{PipeFlags, pipe_with};
use rustix::process::{Pid, WaitOptions, waitpid};
use rustix::stdio;
use wllib::error::SysResultExt;
use wllib::fmt_lite::StringOnStack;

use crate::cli::Config;
use crate::error::AppError;

fn exec_and_read<const N: usize>(
  argv: [*const c_char; N],
  buffer: &mut [u8],
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
        if offset == buffer.len() {
          break Ok(());
        }
        match read(&read_fd, &mut buffer[offset..]) {
          Ok(0) => break Ok(()),
          Ok(count) => offset += count,
          Err(Errno::INTR) => continue,
          Err(error) => break Err(error),
        }
      };

      // Always close the pipe and reap the child, including after a read error.
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
      result.syserr("read")?;

      if offset != buffer.len() {
        return Err(AppError::ImageDecodeError);
      }
    }
  }
  Ok(())
}

fn load_and_scale(
  out_width: u32,
  out_height: u32,
  buffer: &mut [u8],
  config: &Config,
) -> Result<(), AppError> {
  let mut filter = StringOnStack::<160>::new();

  filter
    .push("scale=")
    .push(out_width)
    .push(":")
    .push(out_height);

  match config.fill {
    true => {
      filter
        .push(":force_original_aspect_ratio=increase,crop=")
        .push(out_width)
        .push(":")
        .push(out_height);
    }
    false => {
      filter
        .push(":force_original_aspect_ratio=decrease,pad=")
        .push(out_width)
        .push(":")
        .push(out_height)
        .push(":(ow-iw)/2:(oh-ih)/2");
    }
  }

  if config.blur != 0 {
    filter.push(",gblur=sigma=").push(config.blur);
  }

  let argv: [*const c_char; 11] = [
    c"ffmpeg".as_ptr(),
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
  exec_and_read(argv, buffer)
}

pub fn get_image_fd(out_width: u32, out_height: u32, config: &Config) -> Result<OwnedFd, AppError> {
  let stride = out_width * 4; // 4 bytes per pixel (BGRA)
  let size = (stride * out_height) as usize;

  let fd = memfd_create(
    c"rustbg-wayland-shm",
    MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING | MemfdFlags::NOEXEC_SEAL,
  )
  .syserr("memfd_create")?;

  ftruncate(&fd, size as u64).syserr("ftruncate")?;

  let mmap_ptr = unsafe {
    mmap(
      core::ptr::null_mut(),
      size,
      ProtFlags::READ | ProtFlags::WRITE,
      MapFlags::SHARED,
      &fd,
      0,
    )
  }
  .syserr("mmap")?;

  // Decode directly into shared memory and unmap it even when decoding fails.
  let mmap_slice = unsafe { slice::from_raw_parts_mut(mmap_ptr.cast::<u8>(), size) };
  let result = load_and_scale(out_width, out_height, mmap_slice, config);
  unsafe { munmap(mmap_ptr, size) }.syserr("munmap")?;
  result?;

  Ok(fd)
}
