//! Direct syscall and process management helpers.

use core::arch::asm;
use core::ffi::c_char;
use core::hint::unreachable_unchecked;

use rustix::io::Errno;
use wllib::fmt_lite::StringOnStack;

use crate::env::{get_env, get_envp};

const SYS_FORK: usize = 57;
const SYS_EXECVE: usize = 59;
const SYS_EXIT_GROUP: usize = 231;
const SYS_RT_SIGACTION: usize = 13;

#[inline(always)]
fn check_ret(ret: isize) -> Result<usize, Errno> {
  if (-4095..0).contains(&ret) {
    Err(Errno::from_raw_os_error(-ret as i32))
  } else {
    Ok(ret as usize)
  }
}

/// # Safety
/// `argv` must be a null-terminated array of valid C string pointers.
pub unsafe fn exec_ffmpeg(argv: *const *const c_char) -> ! {
  for dir in get_env("PATH").unwrap_or("/usr/bin:/bin").split(':') {
    let dir = if dir.is_empty() {
      "."
    } else {
      dir.trim_end_matches('/')
    };
    let mut candidate = StringOnStack::<256>::new();
    if dir.len() + "/ffmpeg".len() > candidate.capacity() {
      continue;
    }
    candidate.push(dir).push("/ffmpeg");
    match unsafe { execve(candidate.as_ptr(), argv) } {
      Errno::ACCESS | Errno::NOENT | Errno::NOTDIR => continue,
      _ => break,
    }
  }
  exit(1)
}

pub enum Fork {
  Child,
  Parent(u32),
}

/// Execute a fork syscall.
pub fn fork() -> Result<Fork, Errno> {
  let ret: isize;
  unsafe {
    asm!(
      "syscall",
      in("rax") SYS_FORK,
      lateout("rax") ret,
      lateout("rcx") _,
      lateout("r11") _,
      options(nostack)
    );
  }

  match check_ret(ret)? {
    0 => Ok(Fork::Child),
    _ => Ok(Fork::Parent(ret as u32)),
  }
}

/// Execute an execve syscall.
///
/// # SAFETY:
/// `path` must point to a null-terminated C string.
/// `argv` must point to a null-terminated array of null-terminated string pointers.
unsafe fn execve(path: *const c_char, argv: *const *const c_char) -> Errno {
  let ret: isize;
  unsafe {
    asm!(
      "syscall",
      in("rax") SYS_EXECVE,
      in("rdi") path,
      in("rsi") argv,
      in("rdx") get_envp(),
      lateout("rax") ret,
      lateout("rcx") _,
      lateout("r11") _,
      options(nostack)
    );
  }
  match check_ret(ret) {
    // only returns errors
    Ok(_) => unsafe { unreachable_unchecked() },
    Err(e) => e,
  }
}

pub fn exit(code: i32) -> ! {
  unsafe {
    asm!("syscall", in("rax") SYS_EXIT_GROUP, in("rdi") code, options(nostack, noreturn));
  }
}

pub fn reset_sigpipe() -> Result<(), Errno> {
  // Linux x86-64 kernel sigaction layout. SIG_DFL needs no restorer.
  let action = [0u64; 4];
  let ret: isize;
  unsafe {
    asm!(
      "syscall",
      in("rax") SYS_RT_SIGACTION,
      in("rdi") 13usize,
      in("rsi") action.as_ptr(),
      in("rdx") 0usize,
      in("r10") size_of::<u64>(),
      lateout("rax") ret,
      lateout("rcx") _,
      lateout("r11") _,
      options(nostack)
    );
  }
  check_ret(ret).map(|_| ())
}
