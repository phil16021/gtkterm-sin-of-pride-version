//! Discovering serial ports and watching for hotplug events.

use std::fmt::Write as _;
use std::os::fd::AsFd;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use nix::fcntl::{OFlag, open};
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use nix::sys::stat::Mode;
use serde::Serialize;
use tokio::sync::mpsc;
use tracing::debug;

/// What kind of device a port is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PortClass {
    /// USB serial converter (`ttyUSB*`).
    UsbSerial,
    /// USB CDC-ACM device (`ttyACM*`), e.g. most microcontroller boards.
    UsbAcm,
    /// On-board or PCI UART (`ttyS*`, `ttyAMA*`, ...).
    Uart,
    /// Bluetooth RFCOMM.
    Bluetooth,
    /// Pseudo terminal (`/dev/pts/*`), e.g. created by socat.
    Pty,
}

/// A discovered port.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PortInfo {
    /// Device node.
    pub path: String,
    /// Device class.
    pub class: PortClass,
    /// Vendor name, if known.
    pub vendor: Option<String>,
    /// Product name, if known.
    pub model: Option<String>,
    /// USB serial number, if any.
    pub serial: Option<String>,
    /// Kernel driver.
    pub driver: Option<String>,
}

impl PortInfo {
    /// One-line label, e.g. `/dev/ttyUSB0 — FTDI FT232R (A50285BI)`.
    pub fn label(&self) -> String {
        let mut s = self.path.clone();
        let desc = [self.vendor.as_deref(), self.model.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
        if !desc.is_empty() {
            s.push_str(" — ");
            s.push_str(&desc);
        }
        if let Some(serial) = &self.serial {
            let _ = write!(s, " ({serial})");
        }
        s
    }
}

fn classify(name: &str) -> Option<PortClass> {
    if name.starts_with("ttyUSB") {
        Some(PortClass::UsbSerial)
    } else if name.starts_with("ttyACM") {
        Some(PortClass::UsbAcm)
    } else if name.starts_with("rfcomm") {
        Some(PortClass::Bluetooth)
    } else if [
        "ttyS",
        "ttyAMA",
        "ttyTHS",
        "ttymxc",
        "ttySAC",
        "ttyO",
        "ttyXRUSB",
        "ttyCH343USB",
    ]
    .iter()
    .any(|p| name.starts_with(p))
    {
        Some(PortClass::Uart)
    } else {
        None
    }
}

/// Legacy 8250 UARTs are always registered (ttyS0-31); keep only those with hardware behind.
fn uart_present(path: &Path) -> bool {
    match open(
        path,
        OFlag::O_RDWR | OFlag::O_NOCTTY | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => crate::port::serial::sys::uart_type(fd.as_fd()).map_or(true, |t| t != 0),
        // No permission (not in group dialout): show it so the user can fix that.
        Err(nix::errno::Errno::EACCES) => true,
        Err(_) => false,
    }
}

fn prop(dev: &udev::Device, key: &str) -> Option<String> {
    dev.property_value(key)
        .map(|v| v.to_string_lossy().replace("\\x20", " ").trim().to_owned())
        .filter(|s| !s.is_empty())
}

/// List serial ports: hardware ports via udev, then accessible PTYs. Sorted by class and path.
pub fn list_ports() -> Vec<PortInfo> {
    let mut ports = Vec::new();
    if let Ok(mut en) = udev::Enumerator::new() {
        let _ = en.match_subsystem("tty");
        if let Ok(devices) = en.scan_devices() {
            for dev in devices {
                let Some(node) = dev.devnode() else { continue };
                let name = dev.sysname().to_string_lossy().into_owned();
                let Some(class) = classify(&name) else { continue };
                if class == PortClass::Uart && name.starts_with("ttyS") && !uart_present(node) {
                    continue;
                }
                ports.push(PortInfo {
                    path: node.to_string_lossy().into_owned(),
                    class,
                    vendor: prop(&dev, "ID_VENDOR_FROM_DATABASE").or_else(|| prop(&dev, "ID_VENDOR")),
                    model: prop(&dev, "ID_MODEL_FROM_DATABASE").or_else(|| prop(&dev, "ID_MODEL")),
                    serial: prop(&dev, "ID_SERIAL_SHORT"),
                    driver: dev
                        .parent()
                        .and_then(|p| p.driver().map(|d| d.to_string_lossy().into_owned())),
                });
            }
        }
    }
    ports.extend(list_ptys());
    ports.sort_by_key(|p| (p.class, natural_key(&p.path)));
    ports.dedup_by(|a, b| a.path == b.path);
    ports
}

/// Accessible `/dev/pts/N` entries (the C version listed these for socat testing).
fn list_ptys() -> Vec<PortInfo> {
    let Ok(dir) = std::fs::read_dir("/dev/pts") else {
        return Vec::new();
    };
    dir.filter_map(Result::ok)
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .bytes()
                .all(|b| b.is_ascii_digit())
        })
        .filter(|e| {
            nix::unistd::access(
                &e.path(),
                nix::unistd::AccessFlags::R_OK | nix::unistd::AccessFlags::W_OK,
            )
            .is_ok()
        })
        .map(|e| PortInfo {
            path: e.path().to_string_lossy().into_owned(),
            class: PortClass::Pty,
            vendor: None,
            model: Some("pseudo terminal".into()),
            serial: None,
            driver: None,
        })
        .collect()
}

/// Sort `/dev/ttyUSB10` after `/dev/ttyUSB9`.
fn natural_key(path: &str) -> (String, u64) {
    let digits = path.len() - path.bytes().rev().take_while(u8::is_ascii_digit).count();
    (path[..digits].to_owned(), path[digits..].parse().unwrap_or(0))
}

/// Watches udev for tty add/remove events. Dropping it stops the watcher thread.
#[derive(Debug)]
pub struct PortWatcher {
    stop: Arc<AtomicBool>,
    /// Receives a notification (debounced) whenever ports appeared or disappeared.
    pub changes: mpsc::Receiver<()>,
}

impl Drop for PortWatcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Start watching for port changes. Returns `None` if udev monitoring is unavailable.
pub fn watch_ports() -> Option<PortWatcher> {
    let (tx, changes) = mpsc::channel(4);
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("udev-watch".into())
        .spawn(move || {
            let socket = match udev::MonitorBuilder::new()
                .and_then(|b| b.match_subsystem("tty"))
                .and_then(udev::MonitorBuilder::listen)
            {
                Ok(s) => {
                    let _ = ready_tx.send(true);
                    s
                }
                Err(e) => {
                    debug!(error = %e, "udev monitor unavailable");
                    let _ = ready_tx.send(false);
                    return;
                }
            };
            while !flag.load(Ordering::Relaxed) {
                let mut fds = [PollFd::new(socket.as_fd(), PollFlags::POLLIN)];
                if !matches!(poll(&mut fds, PollTimeout::from(500u16)), Ok(n) if n > 0) {
                    continue;
                }
                let mut any = false;
                for _ in socket.iter() {
                    any = true;
                }
                if any {
                    // Let udev finish creating symlinks/permissions before listing.
                    std::thread::sleep(Duration::from_millis(300));
                    for _ in socket.iter() {}
                    if tx.try_send(()).is_err() && tx.is_closed() {
                        return;
                    }
                }
            }
        })
        .ok()?;
    ready_rx.recv().ok().filter(|ok| *ok)?;
    Some(PortWatcher { stop, changes })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification_and_sorting() {
        assert_eq!(classify("ttyUSB0"), Some(PortClass::UsbSerial));
        assert_eq!(classify("ttyACM3"), Some(PortClass::UsbAcm));
        assert_eq!(classify("ttyS1"), Some(PortClass::Uart));
        assert_eq!(classify("tty0"), None);
        assert_eq!(classify("console"), None);
        assert!(natural_key("/dev/ttyUSB9") < natural_key("/dev/ttyUSB10"));
    }

    #[test]
    fn labels() {
        let p = PortInfo {
            path: "/dev/ttyUSB0".into(),
            class: PortClass::UsbSerial,
            vendor: Some("FTDI".into()),
            model: Some("FT232R".into()),
            serial: Some("A5".into()),
            driver: None,
        };
        assert_eq!(p.label(), "/dev/ttyUSB0 — FTDI FT232R (A5)");
    }

    #[tokio::test]
    async fn listing_includes_a_fresh_pty() {
        let pty = crate::testing::PtyPair::new().unwrap();
        let ports = list_ports();
        assert!(
            ports
                .iter()
                .any(|p| p.path == pty.path_str() && p.class == PortClass::Pty),
            "{ports:?}"
        );
    }
}
