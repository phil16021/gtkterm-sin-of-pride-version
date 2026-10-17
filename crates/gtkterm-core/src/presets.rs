//! Serial line presets: named baud rate / framing / flow control combinations.
//!
//! Built-in presets are the ten of the C version; user presets live in `config.toml` under
//! `[presets.<name>]`. The C JSON format (`{"presets":[{"name","description","config":{...}}]}`)
//! can be imported and exported.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::config::{FlowControl, Parity, SerialConfig};

/// Line settings without the device path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SerialPreset {
    /// What it is for.
    #[serde(default)]
    pub description: String,
    /// Baud rate.
    pub baudrate: u32,
    /// Data bits.
    #[serde(default = "default_bits")]
    pub bits: u8,
    /// Parity.
    #[serde(default)]
    pub parity: Parity,
    /// Stop bits (1 or 2).
    #[serde(default = "default_stop")]
    pub stopbits: u8,
    /// Flow control.
    #[serde(default)]
    pub flow_control: FlowControl,
}

fn default_bits() -> u8 {
    8
}

fn default_stop() -> u8 {
    1
}

impl SerialPreset {
    /// Apply to a serial configuration; the device path and RS485 timings stay.
    pub fn apply(&self, cfg: &mut SerialConfig) {
        cfg.baudrate = self.baudrate;
        cfg.bits = self.bits;
        cfg.parity = self.parity;
        cfg.stopbits = self.stopbits;
        cfg.flow_control = self.flow_control;
    }

    /// Take the line settings of a serial configuration.
    pub fn from_serial(cfg: &SerialConfig, description: &str) -> Self {
        Self {
            description: description.to_owned(),
            baudrate: cfg.baudrate,
            bits: cfg.bits,
            parity: cfg.parity,
            stopbits: cfg.stopbits,
            flow_control: cfg.flow_control,
        }
    }

    /// Short form, e.g. `19200 8E1`.
    pub fn summary(&self) -> String {
        format!(
            "{} {}{}{}",
            self.baudrate,
            self.bits,
            self.parity.letter(),
            self.stopbits
        )
    }
}

/// The C version's built-in presets, in its order.
pub fn builtin_presets() -> Vec<(String, SerialPreset)> {
    use FlowControl::{None as NoFlow, RtsCts};
    use Parity::{Even, None as NoParity};
    let table: [(&str, &str, u32, u8, Parity, FlowControl); 10] = [
        (
            "Arduino",
            "Standard Arduino configuration",
            115_200,
            8,
            NoParity,
            NoFlow,
        ),
        (
            "ESP32/ESP8266",
            "Espressif ESP boards",
            115_200,
            8,
            NoParity,
            NoFlow,
        ),
        ("GPS Module", "Standard GPS module", 9600, 8, NoParity, NoFlow),
        (
            "HC-05 Bluetooth",
            "HC-05 Bluetooth module",
            9600,
            8,
            NoParity,
            NoFlow,
        ),
        (
            "Modbus RTU",
            "Industrial Modbus protocol",
            19_200,
            8,
            Even,
            NoFlow,
        ),
        (
            "Modem (9600)",
            "Classic modem with hardware flow control",
            9600,
            8,
            NoParity,
            RtsCts,
        ),
        (
            "Raspberry Pi",
            "Raspberry Pi serial console",
            115_200,
            8,
            NoParity,
            NoFlow,
        ),
        ("STM32", "STM32 microcontroller", 115_200, 8, NoParity, NoFlow),
        ("Legacy System", "Old industrial equipment", 2400, 7, Even, NoFlow),
        ("High Speed", "High-speed debugging", 921_600, 8, NoParity, RtsCts),
    ];
    table
        .into_iter()
        .map(|(name, desc, baudrate, bits, parity, flow_control)| {
            (
                name.to_owned(),
                SerialPreset {
                    description: desc.to_owned(),
                    baudrate,
                    bits,
                    parity,
                    stopbits: 1,
                    flow_control,
                },
            )
        })
        .collect()
}

/// All presets: built-ins first, then user presets (a user preset with a built-in's name wins).
pub fn all_presets(user: &BTreeMap<String, SerialPreset>) -> Vec<(String, SerialPreset, bool)> {
    let mut out: Vec<(String, SerialPreset, bool)> = builtin_presets()
        .into_iter()
        .filter(|(n, _)| !user.contains_key(n))
        .map(|(n, p)| (n, p, true))
        .collect();
    out.extend(user.iter().map(|(n, p)| (n.clone(), p.clone(), false)));
    out
}

/// Find a preset by name (case-insensitive).
pub fn find_preset(user: &BTreeMap<String, SerialPreset>, name: &str) -> Option<SerialPreset> {
    all_presets(user)
        .into_iter()
        .find(|(n, _, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, p, _)| p)
}

/// Preset import errors.
#[derive(Debug, Error)]
pub enum PresetError {
    /// Malformed JSON.
    #[error("invalid presets file: {0}")]
    Json(#[from] serde_json::Error),
    /// A preset that cannot be represented.
    #[error("preset {name:?}: {message}")]
    Invalid {
        /// Preset name.
        name: String,
        /// Reason.
        message: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct CFile {
    presets: Vec<CPreset>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CPreset {
    name: String,
    #[serde(default)]
    description: String,
    config: CConfig,
}

#[derive(Debug, Serialize, Deserialize)]
struct CConfig {
    baud_rate: u32,
    #[serde(default = "default_bits")]
    data_bits: u8,
    #[serde(default)]
    parity: String,
    #[serde(default = "default_stop_str")]
    stop_bits: String,
    #[serde(default)]
    flow_control: String,
}

fn default_stop_str() -> String {
    "1".into()
}

/// Import presets in the C JSON format.
pub fn import_c_presets(text: &str) -> Result<Vec<(String, SerialPreset)>, PresetError> {
    let file: CFile = serde_json::from_str(text)?;
    file.presets
        .into_iter()
        .map(|p| {
            let invalid = |message: String| PresetError::Invalid {
                name: p.name.clone(),
                message,
            };
            let stopbits = match p.config.stop_bits.trim() {
                "1" | "" => 1,
                "2" => 2,
                "1.5" => {
                    return Err(invalid(
                        "1.5 stop bits are not supported by Linux serial drivers".into(),
                    ));
                }
                other => return Err(invalid(format!("unknown stop bits {other:?}"))),
            };
            let parity = Parity::parse(if p.config.parity.is_empty() {
                "none"
            } else {
                &p.config.parity
            })
            .ok_or_else(|| invalid(format!("unknown parity {:?}", p.config.parity)))?;
            let flow_control = match p.config.flow_control.to_ascii_lowercase().as_str() {
                "" | "none" => FlowControl::None,
                "xonxoff" | "xon" => FlowControl::XonXoff,
                "rtscts" | "rts" => FlowControl::RtsCts,
                "rs485" => FlowControl::Rs485,
                "dtrdsr" => return Err(invalid("DTR/DSR flow control is not supported".into())),
                other => return Err(invalid(format!("unknown flow control {other:?}"))),
            };
            if !(5..=8).contains(&p.config.data_bits) {
                return Err(invalid(format!("{} data bits", p.config.data_bits)));
            }
            Ok((
                p.name.clone(),
                SerialPreset {
                    description: p.description,
                    baudrate: p.config.baud_rate,
                    bits: p.config.data_bits,
                    parity,
                    stopbits,
                    flow_control,
                },
            ))
        })
        .collect()
}

/// Export presets in the C JSON format.
pub fn export_c_presets(presets: &[(String, SerialPreset)]) -> String {
    let file = CFile {
        presets: presets
            .iter()
            .map(|(name, p)| CPreset {
                name: name.clone(),
                description: p.description.clone(),
                config: CConfig {
                    baud_rate: p.baudrate,
                    data_bits: p.bits,
                    parity: format!("{:?}", p.parity).to_lowercase(),
                    stop_bits: p.stopbits.to_string(),
                    flow_control: match p.flow_control {
                        FlowControl::None => "none",
                        FlowControl::XonXoff => "xonxoff",
                        FlowControl::RtsCts => "rtscts",
                        FlowControl::Rs485 => "rs485",
                    }
                    .into(),
                },
            })
            .collect(),
    };
    serde_json::to_string_pretty(&file).unwrap_or_default() + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Connection, Profile};

    #[test]
    fn builtins_are_valid_and_complete() {
        let b = builtin_presets();
        assert_eq!(b.len(), 10);
        for (name, p) in &b {
            let mut prof = Profile::default();
            p.apply(prof.connection.serial_mut());
            prof.validate().unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        let modbus = find_preset(&BTreeMap::new(), "modbus rtu").unwrap();
        assert_eq!(modbus.summary(), "19200 8E1");
        assert_eq!(
            find_preset(&BTreeMap::new(), "Legacy System").unwrap().summary(),
            "2400 7E1"
        );
    }

    #[test]
    fn apply_keeps_port_and_rs485_timing() {
        let mut cfg = SerialConfig {
            port: "/dev/ttyUSB3".into(),
            rs485_rts_time_before_tx: 7,
            ..SerialConfig::default()
        };
        find_preset(&BTreeMap::new(), "High Speed")
            .unwrap()
            .apply(&mut cfg);
        assert_eq!(
            (
                cfg.port.as_str(),
                cfg.baudrate,
                cfg.flow_control,
                cfg.rs485_rts_time_before_tx
            ),
            ("/dev/ttyUSB3", 921_600, FlowControl::RtsCts, 7)
        );
        let back = SerialPreset::from_serial(&cfg, "x");
        assert_eq!(back.baudrate, 921_600);
        let _ = Connection::Serial(cfg);
    }

    #[test]
    fn user_presets_override_and_list_order() {
        let mut user = BTreeMap::new();
        user.insert(
            "Arduino".to_owned(),
            SerialPreset {
                description: "mine".into(),
                baudrate: 57_600,
                bits: 8,
                parity: Parity::None,
                stopbits: 1,
                flow_control: FlowControl::None,
            },
        );
        user.insert(
            "Zeta".to_owned(),
            SerialPreset {
                description: String::new(),
                baudrate: 300,
                bits: 7,
                parity: Parity::Odd,
                stopbits: 2,
                flow_control: FlowControl::None,
            },
        );
        let all = all_presets(&user);
        assert_eq!(all.len(), 11);
        assert_eq!(find_preset(&user, "arduino").unwrap().baudrate, 57_600);
        assert!(!all.last().unwrap().2, "user presets are listed last and marked");
    }

    #[test]
    fn c_json_roundtrip_and_rejections() {
        let json = export_c_presets(&builtin_presets());
        assert_eq!(import_c_presets(&json).unwrap(), builtin_presets());
        let c = r#"{"presets":[{"name":"Odd","description":"x","config":{"baud_rate":4800,"data_bits":7,"parity":"mark","stop_bits":"2","flow_control":"xonxoff"}}]}"#;
        let p = &import_c_presets(c).unwrap()[0].1;
        assert_eq!(
            (p.parity, p.stopbits, p.flow_control),
            (Parity::Mark, 2, FlowControl::XonXoff)
        );
        let bad = r#"{"presets":[{"name":"Half","config":{"baud_rate":9600,"stop_bits":"1.5"}}]}"#;
        assert!(
            import_c_presets(bad)
                .unwrap_err()
                .to_string()
                .contains("1.5 stop bits")
        );
        let dtr = r#"{"presets":[{"name":"D","config":{"baud_rate":9600,"flow_control":"dtrdsr"}}]}"#;
        assert!(import_c_presets(dtr).is_err());
    }
}
