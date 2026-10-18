//! P4: the C writer ignored short writes, so data was lost when the receiver was slow. A PTY
//! whose other end reads slowly forces partial writes and EAGAIN; every byte must arrive in order.

use std::time::Duration;

use gtkterm_core::config::{Connection, SerialConfig, TerminalConfig};
use gtkterm_core::session::Session;
use gtkterm_core::testing::PtyPair;

#[tokio::test]
async fn c_p4_slow_reader_gets_every_byte() {
    const TOTAL: usize = 512 * 1024;
    let pty = PtyPair::new().unwrap();
    let s = Session::spawn(1, "p4", TerminalConfig::default());
    s.open(Connection::Serial(SerialConfig {
        port: pty.path_str(),
        ..SerialConfig::default()
    }))
    .await
    .unwrap();
    // Not text: a counter pattern, so lost or reordered pieces show up.
    let data: Vec<u8> = (0..TOTAL).map(|i| (i % 251) as u8).collect();
    let sender = {
        let s = s.clone();
        let data = data.clone();
        tokio::spawn(async move { s.send(data).await })
    };
    let mut got = Vec::with_capacity(TOTAL);
    let read = async {
        while got.len() < TOTAL {
            got.extend(pty.read().await.unwrap());
            if got.len() % (64 * 1024) < 4096 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(30), read)
        .await
        .expect("all data within 30 s");
    sender.await.unwrap().unwrap();
    assert_eq!(got.len(), TOTAL);
    let first_bad = got.iter().zip(&data).position(|(a, b)| a != b);
    assert_eq!(first_bad, None, "bytes differ");
    assert_eq!(s.status().tx_bytes, TOTAL as u64);
}
