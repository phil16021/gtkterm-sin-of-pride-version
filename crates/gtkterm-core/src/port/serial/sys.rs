//! Thin wrappers around the Linux serial ioctls that `nix` does not cover.
//!
//! This is the only module in the workspace that uses `unsafe`. Every call passes a valid,
//! open file descriptor borrowed for the duration of the call and a pointer to a properly
//! sized, initialised `repr(C)` value that lives on the stack for the whole call.
#![allow(unsafe_code)]

use std::io;
use std::os::fd::{AsRawFd, BorrowedFd, OwnedFd};

use tokio::io::unix::AsyncFd;

pub(crate) use libc::termios2 as Termios2;

/// Register an owned descriptor with the tokio reactor. Must run inside a tokio runtime.
pub(crate) fn async_fd(fd: OwnedFd) -> io::Result<AsyncFd<OwnedFd>> {
    // SAFETY: `OwnedFd` owns an open descriptor, keeps it open until the `AsyncFd` drops it,
    // and always returns the same raw fd, which is exactly the contract of `register`.
    Ok(unsafe { AsyncFd::register(fd) }?)
}

fn check(ret: libc::c_int) -> io::Result<()> {
    if ret == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// `TCGETS2`: read termios including arbitrary speeds.
pub(crate) fn tcgets2(fd: BorrowedFd<'_>) -> io::Result<Termios2> {
    // SAFETY: termios2 is plain old data; all-zero is a valid bit pattern.
    let mut t: Termios2 = unsafe { std::mem::zeroed() };
    // SAFETY: fd is open; TCGETS2 writes exactly one termios2 into `t`.
    check(unsafe { libc::ioctl(fd.as_raw_fd(), libc::TCGETS2, &raw mut t) })?;
    Ok(t)
}

/// `TCSETS2`: apply termios immediately.
pub(crate) fn tcsets2(fd: BorrowedFd<'_>, t: &Termios2) -> io::Result<()> {
    // SAFETY: fd is open; TCSETS2 only reads one termios2 from `t`.
    check(unsafe { libc::ioctl(fd.as_raw_fd(), libc::TCSETS2, std::ptr::from_ref(t)) })
}

/// `TIOCMGET`: modem line bits.
pub(crate) fn tiocmget(fd: BorrowedFd<'_>) -> io::Result<libc::c_int> {
    let mut bits: libc::c_int = 0;
    // SAFETY: fd is open; TIOCMGET writes one c_int.
    check(unsafe { libc::ioctl(fd.as_raw_fd(), libc::TIOCMGET, &raw mut bits) })?;
    Ok(bits)
}

/// `TIOCMBIS` / `TIOCMBIC`: set or clear modem line bits.
pub(crate) fn tiocm_set(fd: BorrowedFd<'_>, bits: libc::c_int, on: bool) -> io::Result<()> {
    let request = if on { libc::TIOCMBIS } else { libc::TIOCMBIC };
    // SAFETY: fd is open; TIOCMBIS/TIOCMBIC read one c_int.
    check(unsafe { libc::ioctl(fd.as_raw_fd(), request, &raw const bits) })
}

/// `TIOCOUTQ`: bytes still waiting in the driver's output queue.
pub(crate) fn output_pending(fd: BorrowedFd<'_>) -> io::Result<libc::c_int> {
    let mut n: libc::c_int = 0;
    // SAFETY: fd is open; TIOCOUTQ writes one c_int.
    check(unsafe { libc::ioctl(fd.as_raw_fd(), libc::TIOCOUTQ, &raw mut n) })?;
    Ok(n)
}

/// `TIOCSBRK` / `TIOCCBRK`: start or stop a break condition.
pub(crate) fn set_break(fd: BorrowedFd<'_>, on: bool) -> io::Result<()> {
    let request = if on { libc::TIOCSBRK } else { libc::TIOCCBRK };
    // SAFETY: fd is open; these ioctls take no argument.
    check(unsafe { libc::ioctl(fd.as_raw_fd(), request) })
}

/// `TIOCMIWAIT`: block until one of the given modem input lines changes.
pub(crate) fn tiocmiwait(fd: BorrowedFd<'_>, mask: libc::c_int) -> io::Result<()> {
    // SAFETY: fd is open; TIOCMIWAIT takes the mask by value.
    check(unsafe { libc::ioctl(fd.as_raw_fd(), libc::TIOCMIWAIT, libc::c_ulong::from(mask as u32)) })
}

/// `struct serial_icounter_struct` from `<linux/serial.h>`.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct SerialIcounter {
    pub cts: libc::c_int,
    pub dsr: libc::c_int,
    pub rng: libc::c_int,
    pub dcd: libc::c_int,
    pub rx: libc::c_int,
    pub tx: libc::c_int,
    pub frame: libc::c_int,
    pub overrun: libc::c_int,
    pub parity: libc::c_int,
    pub brk: libc::c_int,
    pub buf_overrun: libc::c_int,
    pub reserved: [libc::c_int; 9],
}

/// `TIOCGICOUNT`: kernel line counters.
pub(crate) fn tiocgicount(fd: BorrowedFd<'_>) -> io::Result<SerialIcounter> {
    let mut c = SerialIcounter::default();
    // SAFETY: fd is open; TIOCGICOUNT writes one serial_icounter_struct.
    check(unsafe { libc::ioctl(fd.as_raw_fd(), libc::TIOCGICOUNT, &raw mut c) })?;
    Ok(c)
}

/// `struct serial_rs485` from `<linux/serial.h>`.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct SerialRs485 {
    pub flags: u32,
    pub delay_rts_before_send: u32,
    pub delay_rts_after_send: u32,
    pub padding: [u32; 5],
}

pub(crate) const SER_RS485_ENABLED: u32 = 1 << 0;
pub(crate) const SER_RS485_RTS_ON_SEND: u32 = 1 << 1;
pub(crate) const SER_RS485_RTS_AFTER_SEND: u32 = 1 << 2;
pub(crate) const SER_RS485_RX_DURING_TX: u32 = 1 << 4;

/// `TIOCSRS485`: configure kernel RS485 direction control.
pub(crate) fn tiocsrs485(fd: BorrowedFd<'_>, cfg: &SerialRs485) -> io::Result<()> {
    let mut copy = *cfg;
    // SAFETY: fd is open; TIOCSRS485 reads (and may update) one serial_rs485.
    check(unsafe { libc::ioctl(fd.as_raw_fd(), libc::TIOCSRS485, &raw mut copy) })
}

/// `flock(LOCK_EX | LOCK_NB)`.
pub(crate) fn flock_exclusive(fd: BorrowedFd<'_>) -> io::Result<()> {
    // SAFETY: fd is open; flock has no memory arguments.
    check(unsafe { libc::flock(fd.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) })
}

/// Release a `flock`. The kernel drops the lock only when the open file is finally released,
/// which can be a moment after `close` returns; unlocking first makes a reopen succeed at once.
pub(crate) fn flock_unlock(fd: BorrowedFd<'_>) -> io::Result<()> {
    // SAFETY: fd is open; flock has no memory arguments.
    check(unsafe { libc::flock(fd.as_raw_fd(), libc::LOCK_UN) })
}

/// `tcdrain`: wait until all output has been transmitted. Blocks.
pub(crate) fn tcdrain(fd: BorrowedFd<'_>) -> io::Result<()> {
    // SAFETY: fd is open; tcdrain has no memory arguments.
    check(unsafe { libc::tcdrain(fd.as_raw_fd()) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn struct_layouts_match_kernel_abi() {
        assert_eq!(std::mem::size_of::<SerialRs485>(), 32);
        assert_eq!(std::mem::size_of::<SerialIcounter>(), 80);
    }
}
