//! Sunshine packet generator and fake devices for manual testing, benchmarks and demos.
//!
//! ```text
//! socat -d -d pty,raw,echo=0,link=/tmp/ttyA pty,raw,echo=0,link=/tmp/ttyB &
//! gtkterm-sunshine -p /tmp/ttyA &
//! sunshine-gen /tmp/ttyB --rate 50 --ids env,B:levels,P:share,S:xy
//! sunshine-gen /tmp/ttyB --device modbus-rtu --unit 1    # Modbus slave
//! sunshine-gen /tmp/ttyB --device at                     # AT modem
//! sunshine-gen /tmp/ttyB --device nmea --rate 1          # GPS
//! ```
//!
//! Targets: a device path, `tcp://HOST:PORT` (connects), `tcp-listen://PORT` or `udp://HOST:PORT`.

use std::f64::consts::TAU;
use std::fmt::Write as _;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::Parser;
use gtkterm_core::config::{Connection, SerialConfig};
use gtkterm_core::port::{self, OpenPort, PortRx};
use gtkterm_protocols::decode::Protocol;
use gtkterm_testkit::devices::{AtModem, ModbusSlave, nmea_fix};

/// What to emulate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum Device {
    /// Sunshine chart packets.
    Sunshine,
    /// Modbus RTU slave (256 registers and coils).
    ModbusRtu,
    /// Modbus ASCII slave.
    ModbusAscii,
    /// Modbus TCP server side (use with tcp-listen://502).
    ModbusTcp,
    /// Hayes AT modem.
    At,
    /// NMEA GPS emitting GGA and RMC (--rate fixes per second).
    Nmea,
}

#[derive(Debug, Parser)]
#[command(about = "Generate Sunshine packets for GTKTerm Sunshine")]
struct Args {
    /// Device path, tcp://HOST:PORT, tcp-listen://PORT or udp://HOST:PORT.
    target: String,
    /// Packets per second (per ID).
    #[arg(long, default_value_t = 20.0)]
    rate: f64,
    /// Comma separated packet IDs, optionally with chart prefixes (L:, B:, P:, S:).
    #[arg(long, default_value = "env,B:levels,P:share,S:xy")]
    ids: String,
    /// Stop after this many rounds (0 = run forever).
    #[arg(long, default_value_t = 0)]
    count: u64,
    /// Baud rate when the target is a serial device.
    #[arg(long, default_value_t = 115_200)]
    baudrate: u32,
    /// Split every write at a random position to exercise reassembly.
    #[arg(long)]
    split: bool,
    /// Every Nth line is garbage or an invalid packet (0 = never).
    #[arg(long, default_value_t = 0)]
    garbage_every: u64,
    /// Also print plain log lines between packets, like real firmware.
    #[arg(long)]
    chatter: bool,
    /// Device to emulate.
    #[arg(long, value_enum, default_value_t = Device::Sunshine)]
    device: Device,
    /// Modbus unit ID.
    #[arg(long, default_value_t = 1)]
    unit: u8,
}

/// Tiny deterministic PRNG (xorshift) so runs are reproducible.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

fn packet(id: &str, t: f64, rng: &mut Rng) -> String {
    let base = id.rsplit(':').next().unwrap_or(id);
    let prefix = id.strip_suffix(base).unwrap_or("");
    match prefix {
        "B:" => format!(
            "{{{id}|Red|255,0,0|{:.1}|Green|0,200,0|{:.1}|Blue|0,0,255|{:.1}}}",
            50.0 + 40.0 * (t * 0.7).sin(),
            50.0 + 40.0 * (t * 1.1).cos(),
            30.0 + 20.0 * rng.unit()
        ),
        "P:" => format!(
            "{{{id}|CPU|255,128,0|{:.1}|IO|0,128,255|{:.1}|Idle|128,128,128|{:.1}}}",
            30.0 + 10.0 * t.sin(),
            15.0 + 5.0 * (t * 2.0).cos(),
            55.0 - 10.0 * t.sin()
        ),
        "S:" => {
            let r = 1.0 + 0.3 * (t * 0.25).sin();
            format!(
                "{{{id}|orbit|200,0,200|{:.3} {:.3}}}",
                r * (t * TAU / 8.0).cos(),
                r * (t * TAU / 8.0).sin()
            )
        }
        _ => format!(
            "{{{id}|Temp|255,0,0|{:.2}|Humidity|0,0,255|{:.2}|Noise|{:.3}}}",
            21.0 + 3.0 * (t * 0.3).sin() + 0.2 * rng.unit(),
            45.0 + 10.0 * (t * 0.13).cos(),
            rng.unit() - 0.5
        ),
    }
}

fn connection(target: &str, baudrate: u32) -> Result<Connection> {
    let host_port = |s: &str| -> Result<(String, u16)> {
        let (h, p) = s.rsplit_once(':').context("expected HOST:PORT")?;
        Ok((h.to_owned(), p.parse().context("bad port")?))
    };
    Ok(if let Some(rest) = target.strip_prefix("tcp://") {
        let (host, port) = host_port(rest)?;
        Connection::TcpClient {
            host,
            port,
            connect_timeout_ms: 5_000,
        }
    } else if let Some(rest) = target.strip_prefix("tcp-listen://") {
        Connection::TcpServer {
            bind: "0.0.0.0".into(),
            port: rest.parse().context("bad port")?,
        }
    } else if let Some(rest) = target.strip_prefix("udp://") {
        let (host, port) = host_port(rest)?;
        Connection::Udp {
            bind: "0.0.0.0".into(),
            local_port: 0,
            remote_host: Some(host),
            remote_port: port,
        }
    } else if target.contains("://") {
        bail!("unknown target scheme in {target}");
    } else {
        Connection::Serial(SerialConfig {
            port: target.to_owned(),
            baudrate,
            disable_port_lock: true,
            ..SerialConfig::default()
        })
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    if args.rate <= 0.0 {
        bail!("--rate must be positive");
    }
    let mut port = port::open(&connection(&args.target, args.baudrate)?)
        .await
        .context("cannot open target")?;
    match args.device {
        Device::Sunshine => {}
        Device::Nmea => return gps(&args, port).await,
        Device::At => {
            let mut modem = AtModem::default();
            eprintln!("AT modem on {}", port.description);
            return respond(port, move |d, _| vec![modem.feed(d)]).await;
        }
        Device::ModbusRtu | Device::ModbusAscii | Device::ModbusTcp => {
            let framing = match args.device {
                Device::ModbusAscii => Protocol::ModbusAscii,
                Device::ModbusTcp => Protocol::ModbusTcp,
                _ => Protocol::ModbusRtu,
            };
            let mut slave = ModbusSlave::new(args.unit, framing);
            eprintln!(
                "Modbus slave (unit {}, {}) on {}; holding register n holds n*10",
                args.unit,
                framing.label(),
                port.description
            );
            return respond(port, move |d, us| {
                slave.poll(us);
                slave.feed(d, us)
            })
            .await;
        }
    }
    eprintln!(
        "sending to {} at {} packets/s per ID",
        port.description, args.rate
    );
    let ids: Vec<&str> = args
        .ids
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut ticker = tokio::time::interval(Duration::from_secs_f64(1.0 / args.rate));
    let mut line_no = 0u64;
    for round in 0.. {
        if args.count != 0 && round >= args.count {
            break;
        }
        ticker.tick().await;
        let t = round as f64 / args.rate;
        let mut out = String::new();
        if args.chatter && round % 10 == 0 {
            let _ = writeln!(out, "[{t:8.2}] heap free: {} bytes\r", 20_000 + rng.next() % 500);
        }
        for id in &ids {
            line_no += 1;
            if args.garbage_every != 0 && line_no.is_multiple_of(args.garbage_every) {
                out.push_str(
                    [
                        "{oops|x|1,5}\r\n",
                        "{broken|y|\r\n",
                        "\u{1}\u{2}garbage\r\n",
                        "{}\r\n",
                    ][(line_no % 4) as usize],
                );
            }
            out.push_str(&packet(id, t, &mut rng));
            out.push_str("\r\n");
        }
        let bytes = out.into_bytes();
        if args.split && bytes.len() > 1 {
            let at = 1 + (rng.next() as usize % (bytes.len() - 1));
            port.writer.write_all(&bytes[..at]).await?;
            tokio::time::sleep(Duration::from_millis(1)).await;
            port.writer.write_all(&bytes[at..]).await?;
        } else {
            port.writer.write_all(&bytes).await?;
        }
    }
    port.close().await;
    Ok(())
}

/// Answer received data until the port closes.
async fn respond(mut port: OpenPort, mut f: impl FnMut(&[u8], i64) -> Vec<Vec<u8>>) -> Result<()> {
    let start = std::time::Instant::now();
    while let Some(rx) = port.rx.recv().await {
        match rx {
            PortRx::Data(d) => {
                let us = i64::try_from(start.elapsed().as_micros()).unwrap_or(i64::MAX);
                for reply in f(&d, us) {
                    if !reply.is_empty() {
                        port.writer.write_all(&reply).await?;
                    }
                }
            }
            PortRx::Info(msg) => eprintln!("[{msg}]"),
            PortRx::Closed(_) => break,
        }
    }
    port.close().await;
    Ok(())
}

async fn gps(args: &Args, mut port: OpenPort) -> Result<()> {
    eprintln!("NMEA GPS on {} at {} fixes/s", port.description, args.rate);
    let mut ticker = tokio::time::interval(Duration::from_secs_f64(1.0 / args.rate));
    for t in 0.. {
        if args.count != 0 && t >= args.count {
            break;
        }
        ticker.tick().await;
        port.writer.write_all(nmea_fix(t).as_bytes()).await?;
    }
    port.close().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use gtkterm_protocols::sunshine::{ChartKind, parse_packet};

    use super::*;

    #[test]
    fn generated_packets_parse() {
        let mut rng = Rng(1);
        for (id, kind) in [
            ("env", None),
            ("B:levels", Some(ChartKind::Bar)),
            ("P:share", Some(ChartKind::Pie)),
            ("S:xy", Some(ChartKind::Scatter)),
        ] {
            for i in 0..100 {
                let line = packet(id, f64::from(i) * 0.1, &mut rng);
                let p = parse_packet(&line).unwrap_or_else(|e| panic!("{line}: {e}"));
                assert_eq!(p.kind, kind);
            }
        }
    }

    #[test]
    fn targets() {
        assert!(matches!(
            connection("tcp://localhost:5000", 0).unwrap(),
            Connection::TcpClient { port: 5000, .. }
        ));
        assert!(matches!(
            connection("udp://h:1", 0).unwrap(),
            Connection::Udp { remote_port: 1, .. }
        ));
        assert!(
            matches!(connection("/dev/ttyUSB0", 9600).unwrap(), Connection::Serial(s) if s.baudrate == 9600)
        );
        assert!(connection("ftp://x", 0).is_err());
    }
}
