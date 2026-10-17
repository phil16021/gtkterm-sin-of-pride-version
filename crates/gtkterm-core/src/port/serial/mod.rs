//! Serial devices on Linux: termios2 (arbitrary baud rates), flock, kernel or software RS485,
//! modem lines and break.
//!
//! Configuration always starts from raw mode with every relevant flag set explicitly, so no
//! stale parity, size or flow bits survive a reconfigure (ROADMAP defect P3). DTR/RTS are set
//! with `TIOCMBIS`/`TIOCMBIC`, never toggled (P1, P2), and writes loop until complete (P4).

pub(crate) mod sys;

use std::os::fd::{AsFd, OwnedFd};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use nix::errno::Errno;
use nix::fcntl::{OFlag, open as nix_open};
use nix::sys::stat::Mode;
use nix::sys::termios::{FlushArg, tcflush};
use tokio::io::unix::AsyncFd;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use super::{
    LineCounters, ModemLines, OpenPort, PortError, PortKind, PortRx, PortWriter, READ_SIZE, RX_CHANNEL,
    SerialControl,
};
use crate::config::{FlowControl, Parity, SerialConfig};

/// Apply `cfg` to a termios2 structure (raw mode, explicit flags, BOTHER speed).
pub(crate) fn configure_termios(t: &mut sys::Termios2, cfg: &SerialConfig) {
    use libc::{
        BOTHER, BRKINT, CBAUD, CIBAUD, CLOCAL, CMSPAR, CREAD, CRTSCTS, CS5, CS6, CS7, CS8, CSIZE, CSTOPB,
        ECHO, ECHONL, ICANON, ICRNL, IEXTEN, IGNBRK, IGNCR, INLCR, INPCK, ISIG, ISTRIP, IXANY, IXOFF, IXON,
        OPOST, PARENB, PARMRK, PARODD, VMIN, VTIME,
    };
    t.c_iflag &= !(IGNBRK | BRKINT | PARMRK | ISTRIP | INLCR | IGNCR | ICRNL | IXON | IXOFF | IXANY | INPCK);
    t.c_oflag &= !OPOST;
    t.c_lflag &= !(ECHO | ECHONL | ICANON | ISIG | IEXTEN);
    t.c_cflag &= !(CSIZE | PARENB | PARODD | CMSPAR | CSTOPB | CRTSCTS | CBAUD | CIBAUD);
    t.c_cflag |= CREAD | CLOCAL | BOTHER;
    t.c_cflag |= match cfg.bits {
        5 => CS5,
        6 => CS6,
        7 => CS7,
        _ => CS8,
    };
    t.c_cflag |= match cfg.parity {
        Parity::None => 0,
        Parity::Odd => PARENB | PARODD,
        Parity::Even => PARENB,
        Parity::Mark => PARENB | CMSPAR | PARODD,
        Parity::Space => PARENB | CMSPAR,
    };
    if cfg.stopbits == 2 {
        t.c_cflag |= CSTOPB;
    }
    match cfg.flow_control {
        FlowControl::XonXoff => t.c_iflag |= IXON | IXOFF,
        FlowControl::RtsCts => t.c_cflag |= CRTSCTS,
        FlowControl::None | FlowControl::Rs485 => {}
    }
    t.c_cc[VMIN] = 1;
    t.c_cc[VTIME] = 0;
    t.c_ispeed = cfg.baudrate;
    t.c_ospeed = cfg.baudrate;
}

/// Shared device state. Dropping it restores the original termios and closes the fd.
#[derive(Debug)]
struct Device {
    fd: AsyncFd<OwnedFd>,
    original: sys::Termios2,
    path: String,
    locked: bool,
}

impl Drop for Device {
    fn drop(&mut self) {
        if let Err(e) = sys::tcsets2(self.fd.get_ref().as_fd(), &self.original) {
            debug!(path = %self.path, error = %e, "could not restore termios");
        }
        if self.locked {
            let _ = sys::flock_unlock(self.fd.get_ref().as_fd());
        }
    }
}

/// Open and configure a serial device. Must run inside a tokio runtime.
pub(crate) fn open(cfg: &SerialConfig) -> Result<OpenPort, PortError> {
    let path = cfg.port.clone();
    let fd = nix_open(
        path.as_str(),
        OFlag::O_RDWR | OFlag::O_NOCTTY | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC,
        Mode::empty(),
    )
    .map_err(|e| PortError::io(format!("cannot open {path}"), e))?;

    if !cfg.disable_port_lock {
        match sys::flock_exclusive(fd.as_fd()) {
            Ok(()) => {}
            Err(e) if e.raw_os_error() == Some(libc::EWOULDBLOCK) => return Err(PortError::Locked(path)),
            Err(e) => return Err(PortError::io(format!("cannot lock {path}"), e)),
        }
    }

    let original =
        sys::tcgets2(fd.as_fd()).map_err(|e| PortError::io(format!("{path} is not a serial device"), e))?;
    let mut termios = original;
    configure_termios(&mut termios, cfg);
    sys::tcsets2(fd.as_fd(), &termios).map_err(|e| PortError::io(format!("cannot configure {path}"), e))?;
    let _ = tcflush(&fd, FlushArg::TCIOFLUSH);

    let rs485_soft = if cfg.flow_control == FlowControl::Rs485 {
        let kernel = sys::SerialRs485 {
            flags: sys::SER_RS485_ENABLED | sys::SER_RS485_RTS_ON_SEND,
            delay_rts_before_send: cfg.rs485_rts_time_before_tx,
            delay_rts_after_send: cfg.rs485_rts_time_after_tx,
            padding: [0; 5],
        };
        match sys::tiocsrs485(fd.as_fd(), &kernel) {
            Ok(()) => {
                debug!(%path, "kernel RS485 mode enabled");
                None
            }
            Err(e) => {
                debug!(%path, error = %e, "kernel RS485 unavailable, using software RTS control");
                let _ = sys::tiocm_set(fd.as_fd(), libc::TIOCM_RTS, false);
                Some((
                    Duration::from_millis(cfg.rs485_rts_time_before_tx.into()),
                    Duration::from_millis(cfg.rs485_rts_time_after_tx.into()),
                ))
            }
        }
    } else {
        None
    };

    let fd = sys::async_fd(fd).map_err(|e| PortError::io(format!("cannot register {path}"), e))?;
    let device = Arc::new(Device {
        fd,
        original,
        path: path.clone(),
        locked: !cfg.disable_port_lock,
    });
    let cancel = CancellationToken::new();
    let (tx, rx) = mpsc::channel(RX_CHANNEL);
    let task = tokio::spawn(read_loop(Arc::clone(&device), tx, cancel.clone()));

    Ok(OpenPort::new(
        PortKind::Serial,
        cfg.summary(),
        rx,
        Box::new(SerialWriter {
            device: Arc::clone(&device),
            rs485_soft,
        }),
        Some(Arc::new(SerialCtl { device })),
        cancel,
        Some(task),
    ))
}

async fn read_loop(device: Arc<Device>, tx: mpsc::Sender<PortRx>, cancel: CancellationToken) {
    let mut buf = vec![0u8; READ_SIZE];
    let reason = loop {
        let mut guard = tokio::select! {
            () = cancel.cancelled() => return,
            r = device.fd.readable() => match r {
                Ok(g) => g,
                Err(e) => break Some(PortError::io("poll failed", e)),
            },
        };
        match guard.try_io(|inner| nix::unistd::read(inner.get_ref(), &mut buf).map_err(std::io::Error::from))
        {
            Ok(Ok(0)) => break None,
            Ok(Ok(n)) => {
                if tx
                    .send(PortRx::Data(Bytes::copy_from_slice(&buf[..n])))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            Ok(Err(e)) if e.raw_os_error() == Some(libc::EINTR) => {}
            Ok(Err(e)) => break Some(PortError::io(format!("read from {} failed", device.path), e)),
            Err(_would_block) => {}
        }
    };
    if let Some(e) = &reason {
        warn!(path = %device.path, error = %e, "serial port closed");
    }
    let _ = tx.send(PortRx::Closed(reason.map(Arc::new))).await;
}

#[derive(Debug)]
struct SerialWriter {
    device: Arc<Device>,
    /// Software RS485: (RTS lead time, RTS tail time) when the kernel cannot do it.
    rs485_soft: Option<(Duration, Duration)>,
}

impl SerialWriter {
    async fn write_raw(&self, mut data: &[u8]) -> Result<(), PortError> {
        while !data.is_empty() {
            let mut guard = self
                .device
                .fd
                .writable()
                .await
                .map_err(|e| PortError::io("poll failed", e))?;
            match guard
                .try_io(|inner| nix::unistd::write(inner.get_ref(), data).map_err(std::io::Error::from))
            {
                Ok(Ok(n)) => data = &data[n..],
                Ok(Err(e)) if e.raw_os_error() == Some(libc::EINTR) => {}
                Ok(Err(e)) => return Err(PortError::io(format!("write to {} failed", self.device.path), e)),
                Err(_would_block) => {}
            }
        }
        Ok(())
    }

    async fn drain(&self) -> Result<(), PortError> {
        let device = Arc::clone(&self.device);
        tokio::task::spawn_blocking(move || sys::tcdrain(device.fd.get_ref().as_fd()))
            .await
            .map_err(|e| PortError::io("tcdrain task failed", std::io::Error::other(e)))?
            .map_err(|e| PortError::io("tcdrain failed", e))
    }
}

#[async_trait]
impl PortWriter for SerialWriter {
    async fn write_all(&mut self, data: &[u8]) -> Result<(), PortError> {
        let Some((before, after)) = self.rs485_soft else {
            return self.write_raw(data).await;
        };
        let fd = self.device.fd.get_ref().as_fd();
        sys::tiocm_set(fd, libc::TIOCM_RTS, true).map_err(|e| PortError::io("cannot assert RTS", e))?;
        tokio::time::sleep(before).await;
        let result = async {
            self.write_raw(data).await?;
            self.drain().await
        }
        .await;
        tokio::time::sleep(after).await;
        let fd = self.device.fd.get_ref().as_fd();
        sys::tiocm_set(fd, libc::TIOCM_RTS, false).map_err(|e| PortError::io("cannot release RTS", e))?;
        result
    }
}

#[derive(Debug)]
struct SerialCtl {
    device: Arc<Device>,
}

fn modem_err(what: &str, e: std::io::Error) -> PortError {
    if matches!(
        Errno::from_raw(e.raw_os_error().unwrap_or(0)),
        Errno::ENOTTY | Errno::EINVAL
    ) {
        PortError::Unsupported(format!("{what} (device has no modem lines)"))
    } else {
        PortError::io(what.to_owned(), e)
    }
}

impl SerialControl for SerialCtl {
    fn modem_lines(&self) -> Result<ModemLines, PortError> {
        let bits =
            sys::tiocmget(self.device.fd.get_ref().as_fd()).map_err(|e| modem_err("read modem lines", e))?;
        Ok(ModemLines {
            dtr: bits & libc::TIOCM_DTR != 0,
            rts: bits & libc::TIOCM_RTS != 0,
            cts: bits & libc::TIOCM_CTS != 0,
            dsr: bits & libc::TIOCM_DSR != 0,
            dcd: bits & libc::TIOCM_CAR != 0,
            ri: bits & libc::TIOCM_RNG != 0,
        })
    }

    fn set_dtr(&self, on: bool) -> Result<(), PortError> {
        sys::tiocm_set(self.device.fd.get_ref().as_fd(), libc::TIOCM_DTR, on)
            .map_err(|e| modem_err("set DTR", e))
    }

    fn set_rts(&self, on: bool) -> Result<(), PortError> {
        sys::tiocm_set(self.device.fd.get_ref().as_fd(), libc::TIOCM_RTS, on)
            .map_err(|e| modem_err("set RTS", e))
    }

    fn set_break(&self, on: bool) -> Result<(), PortError> {
        sys::set_break(self.device.fd.get_ref().as_fd(), on).map_err(|e| modem_err("send break", e))
    }

    fn counters(&self) -> Result<LineCounters, PortError> {
        let c =
            sys::tiocgicount(self.device.fd.get_ref().as_fd()).map_err(|e| modem_err("read counters", e))?;
        let u = |v: libc::c_int| u64::try_from(v).unwrap_or(0);
        Ok(LineCounters {
            rx: u(c.rx),
            tx: u(c.tx),
            frame: u(c.frame),
            overrun: u(c.overrun),
            parity: u(c.parity),
            brk: u(c.brk),
            buf_overrun: u(c.buf_overrun),
        })
    }

    fn wait_modem_change(&self) -> Result<(), PortError> {
        let mask = libc::TIOCM_CTS | libc::TIOCM_DSR | libc::TIOCM_CAR | libc::TIOCM_RNG;
        sys::tiocmiwait(self.device.fd.get_ref().as_fd(), mask)
            .map_err(|e| modem_err("wait for modem lines", e))
    }
}

/// Raw termios flags and output speed of a tty, for tests and diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TermiosSnapshot {
    /// Input flags.
    pub iflag: u32,
    /// Output flags.
    pub oflag: u32,
    /// Control flags.
    pub cflag: u32,
    /// Local flags.
    pub lflag: u32,
    /// Output speed in baud.
    pub ospeed: u32,
}

/// Read back the termios2 of an arbitrary tty fd.
pub fn read_termios(fd: impl AsFd) -> std::io::Result<TermiosSnapshot> {
    let t = sys::tcgets2(fd.as_fd())?;
    Ok(TermiosSnapshot {
        iflag: t.c_iflag,
        oflag: t.c_oflag,
        cflag: t.c_cflag,
        lflag: t.c_lflag,
        ospeed: t.c_ospeed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cooked() -> sys::Termios2 {
        // A typical cooked tty with leftover flags from an earlier 7O2 RTS/CTS setup.
        let mut t = sys::Termios2 {
            c_iflag: libc::ICRNL | libc::IXON | libc::INPCK,
            c_oflag: libc::OPOST,
            c_cflag: libc::CS7 | libc::PARENB | libc::PARODD | libc::CSTOPB | libc::CRTSCTS | libc::B38400,
            c_lflag: libc::ICANON | libc::ECHO | libc::ISIG,
            c_line: 0,
            c_cc: [0; 19],
            c_ispeed: 38_400,
            c_ospeed: 38_400,
        };
        t.c_cc[libc::VMIN] = 0;
        t
    }

    #[test]
    fn reconfigure_clears_stale_bits() {
        // C defect P3: switching from 7O2 RTS/CTS to 8N1 none must clear every old flag.
        let mut t = cooked();
        configure_termios(
            &mut t,
            &SerialConfig {
                baudrate: 250_000,
                ..SerialConfig::default()
            },
        );
        assert_eq!(t.c_cflag & libc::CSIZE, libc::CS8);
        assert_eq!(
            t.c_cflag & (libc::PARENB | libc::PARODD | libc::CSTOPB | libc::CRTSCTS),
            0
        );
        assert_eq!(t.c_cflag & libc::CBAUD, libc::BOTHER);
        assert_eq!((t.c_ispeed, t.c_ospeed), (250_000, 250_000));
        assert_eq!(t.c_iflag & (libc::ICRNL | libc::IXON | libc::INPCK), 0);
        assert_eq!(t.c_oflag & libc::OPOST, 0);
        assert_eq!(t.c_lflag & (libc::ICANON | libc::ECHO | libc::ISIG), 0);
        assert_eq!(t.c_cc[libc::VMIN], 1);
    }

    #[test]
    fn parity_stop_and_flow_flags() {
        let mut t = cooked();
        let cfg = SerialConfig {
            bits: 7,
            stopbits: 2,
            parity: Parity::Mark,
            flow_control: FlowControl::XonXoff,
            ..SerialConfig::default()
        };
        configure_termios(&mut t, &cfg);
        assert_eq!(t.c_cflag & libc::CSIZE, libc::CS7);
        assert_ne!(t.c_cflag & libc::CSTOPB, 0);
        assert_eq!(
            t.c_cflag & (libc::PARENB | libc::CMSPAR | libc::PARODD),
            libc::PARENB | libc::CMSPAR | libc::PARODD
        );
        assert_eq!(t.c_iflag & (libc::IXON | libc::IXOFF), libc::IXON | libc::IXOFF);
        assert_eq!(t.c_cflag & libc::CRTSCTS, 0);

        configure_termios(
            &mut t,
            &SerialConfig {
                parity: Parity::Space,
                flow_control: FlowControl::RtsCts,
                ..cfg
            },
        );
        assert_eq!(
            t.c_cflag & (libc::PARENB | libc::CMSPAR | libc::PARODD),
            libc::PARENB | libc::CMSPAR
        );
        assert_ne!(t.c_cflag & libc::CRTSCTS, 0);
        assert_eq!(t.c_iflag & libc::IXON, 0);
    }
}
