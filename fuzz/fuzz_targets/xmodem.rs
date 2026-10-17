//! XMODEM/YMODEM receivers and senders survive arbitrary line noise: no panic, bounded memory,
//! and a finished receiver never reports more data than it was sent.
#![no_main]
use gtkterm_protocols::xmodem::{Event, FileData, Receiver, Sender, Timing, Transfer, Variant};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((&sel, data)) = data.split_first() else { return };
    let variant = Variant::ALL[usize::from(sel) % 4];
    let mut rx = Receiver::new(variant, Timing::default(), "f");
    let mut tx = Sender::new(
        variant,
        vec![FileData { name: "a".into(), data: vec![7; 300], mtime: Some(1) }],
        Timing::default(),
    )
    .unwrap();
    rx.start(0);
    tx.start(0);
    let mut now = 0;
    for chunk in data.chunks(1 + usize::from(sel >> 4)) {
        now += 500;
        for m in [&mut rx as &mut dyn Transfer, &mut tx] {
            let out = m.input(chunk, now);
            for e in out.events {
                if let Event::FileReceived(f) = e {
                    assert!(f.data.len() <= data.len());
                }
            }
            m.drained(now);
            if m.deadline().is_some_and(|d| d <= now) {
                m.tick(now);
            }
        }
    }
    let _ = rx.cancel();
    let _ = tx.cancel();
});
