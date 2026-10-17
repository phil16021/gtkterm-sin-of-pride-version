//! Every decoder survives arbitrary input split at an arbitrary point, and line-based decoders
//! produce the same frames however the input is split.
#![no_main]
use gtkterm_protocols::decode::{Protocol, Role, decoder, detect};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((&split, data)) = data.split_first() else { return };
    let split = usize::from(split).min(data.len());
    let _ = detect(data);
    for protocol in Protocol::ALL {
        for role in [Role::Request, Role::Response] {
            let mut whole = decoder(protocol, role, Some(9600));
            let mut a = whole.feed(data, 0);
            a.extend(whole.poll(1_000_000));
            let mut parts = decoder(protocol, role, Some(9600));
            let mut b = parts.feed(&data[..split], 0);
            b.extend(parts.feed(&data[split..], 1));
            b.extend(parts.poll(1_000_000));
            let line_based = matches!(
                protocol,
                Protocol::At | Protocol::Nmea | Protocol::Can | Protocol::Slcan | Protocol::ModbusAscii
            );
            if line_based {
                assert_eq!(a, b, "{protocol:?} depends on how the input is split");
            }
        }
    }
});
