//! P2: the C DTR/RTS menu toggles did nothing. The session sets (not toggles) the lines on the
//! port's `SerialControl`; the loopback backend records what the host drives.

use gtkterm_core::config::TerminalConfig;
use gtkterm_core::port::loopback;
use gtkterm_core::session::Session;

#[tokio::test]
async fn c_p2_dtr_and_rts_requests_reach_the_lines() {
    let s = Session::spawn(1, "p2", TerminalConfig::default());
    let (port, dev) = loopback::pair_with_lines("loop");
    s.attach(port).await.unwrap();
    for (dtr, rts) in [
        (true, false),
        (true, true),
        (false, true),
        (false, false),
        (false, false),
    ] {
        s.set_dtr(dtr).await.unwrap();
        s.set_rts(rts).await.unwrap();
        let lines = dev.lines();
        assert_eq!(
            (lines.dtr, lines.rts),
            (dtr, rts),
            "after setting DTR={dtr} RTS={rts}"
        );
    }
}
