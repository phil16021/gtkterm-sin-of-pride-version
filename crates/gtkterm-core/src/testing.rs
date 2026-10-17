//! Helpers for tests in this and dependent crates.

use std::os::fd::{AsFd, OwnedFd};
use std::path::PathBuf;

use nix::pty::{OpenptyResult, openpty};
use tokio::io::unix::AsyncFd;

/// A pseudo terminal pair. The slave path behaves like a serial device for [`crate::port`];
/// the master side is the "device" in tests.
#[derive(Debug)]
pub struct PtyPair {
    master: AsyncFd<OwnedFd>,
    /// Keeps the slave open so the master does not see EIO before the port opens.
    slave: Option<OwnedFd>,
    /// Path of the slave, e.g. `/dev/pts/7`.
    pub path: PathBuf,
}

impl PtyPair {
    /// Create a pair. Must run inside a tokio runtime.
    pub fn new() -> std::io::Result<Self> {
        let OpenptyResult { master, slave } = openpty(None, None)?;
        let path = nix::unistd::ttyname(&slave)?;
        let flags = nix::fcntl::fcntl(&master, nix::fcntl::FcntlArg::F_GETFL)?;
        let mut flags = nix::fcntl::OFlag::from_bits_truncate(flags);
        flags.insert(nix::fcntl::OFlag::O_NONBLOCK);
        nix::fcntl::fcntl(&master, nix::fcntl::FcntlArg::F_SETFL(flags))?;
        // Put the slave in raw mode so the line discipline does not echo or translate.
        let mut t = nix::sys::termios::tcgetattr(&slave)?;
        nix::sys::termios::cfmakeraw(&mut t);
        nix::sys::termios::tcsetattr(&slave, nix::sys::termios::SetArg::TCSANOW, &t)?;
        Ok(Self {
            master: crate::port::serial::sys::async_fd(master)?,
            slave: Some(slave),
            path,
        })
    }

    /// Slave path as a string.
    pub fn path_str(&self) -> String {
        self.path.to_string_lossy().into_owned()
    }

    /// Close the extra slave fd (after the port under test has opened it).
    pub fn release_slave(&mut self) {
        self.slave = None;
    }

    /// The slave fd kept by the pair, for termios readback.
    pub fn slave_fd(&self) -> Option<impl AsFd + '_> {
        self.slave.as_ref()
    }

    /// Write all bytes as the device.
    pub async fn write(&self, mut data: &[u8]) -> std::io::Result<()> {
        while !data.is_empty() {
            let mut guard = self.master.writable().await?;
            match guard.try_io(|m| nix::unistd::write(m.get_ref(), data).map_err(std::io::Error::from)) {
                Ok(Ok(n)) => data = &data[n..],
                Ok(Err(e)) => return Err(e),
                Err(_) => {}
            }
        }
        Ok(())
    }

    /// Read whatever the host sent (waits for at least one byte).
    pub async fn read(&self) -> std::io::Result<Vec<u8>> {
        let mut buf = vec![0u8; 65_536];
        loop {
            let mut guard = self.master.readable().await?;
            match guard.try_io(|m| nix::unistd::read(m.get_ref(), &mut buf).map_err(std::io::Error::from)) {
                Ok(Ok(n)) => return Ok(buf[..n].to_vec()),
                Ok(Err(e)) => return Err(e),
                Err(_) => {}
            }
        }
    }

    /// Read exactly `n` bytes.
    pub async fn read_exact(&self, n: usize) -> std::io::Result<Vec<u8>> {
        let mut out = Vec::with_capacity(n);
        while out.len() < n {
            out.extend(self.read().await?);
        }
        Ok(out)
    }

    /// Hang up: close the master side.
    pub fn hang_up(self) {}
}
