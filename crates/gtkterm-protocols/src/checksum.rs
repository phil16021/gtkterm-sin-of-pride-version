//! Checksums used by the decoders and file-transfer protocols.

use crc::{CRC_16_MODBUS, CRC_16_XMODEM, Crc};

const MODBUS: Crc<u16> = Crc::<u16>::new(&CRC_16_MODBUS);
const XMODEM: Crc<u16> = Crc::<u16>::new(&CRC_16_XMODEM);

/// Modbus RTU CRC-16 (poly 0xA001 reflected, init 0xFFFF). Transmitted low byte first.
pub fn crc16_modbus(data: &[u8]) -> u16 {
    MODBUS.checksum(data)
}

/// XMODEM CRC-16/CCITT (poly 0x1021, init 0). Transmitted high byte first.
pub fn crc16_xmodem(data: &[u8]) -> u16 {
    XMODEM.checksum(data)
}

/// 8-bit arithmetic sum, used by classic XMODEM.
pub fn sum8(data: &[u8]) -> u8 {
    data.iter().fold(0u8, |acc, &b| acc.wrapping_add(b))
}

/// Modbus ASCII LRC: two's complement of the 8-bit sum.
pub fn lrc(data: &[u8]) -> u8 {
    sum8(data).wrapping_neg()
}

/// NMEA 0183 checksum: XOR of all bytes between `$`/`!` and `*`.
pub fn nmea_xor(data: &[u8]) -> u8 {
    data.iter().fold(0u8, |acc, &b| acc ^ b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_vectors() {
        assert_eq!(crc16_xmodem(b"123456789"), 0x31C3);
        assert_eq!(crc16_modbus(b"123456789"), 0x4B37);
        // Modbus RTU frame 01 03 00 00 00 0A -> CRC C5 CD on the wire (low byte first).
        assert_eq!(
            crc16_modbus(&[0x01, 0x03, 0x00, 0x00, 0x00, 0x0A]).to_le_bytes(),
            [0xC5, 0xCD]
        );
        // Modbus ASCII ":010300000001FB": LRC over 01 03 00 00 00 01.
        assert_eq!(lrc(&[0x01, 0x03, 0x00, 0x00, 0x00, 0x01]), 0xFB);
        // Classic example sentence `$GPGLL,4916.45,N,12311.12,W,225444,A,*1D`.
        assert_eq!(nmea_xor(b"GPGLL,4916.45,N,12311.12,W,225444,A,"), 0x1D);
        assert_eq!(sum8(&[0xFF, 0x02]), 0x01);
    }
}
