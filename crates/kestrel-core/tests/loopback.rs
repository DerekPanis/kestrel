//! Exercises the real libchiaki code paths over loopback, without a console.
//! These tests prove the bridge, threading and callbacks work end to end;
//! they do NOT replace validation against real PS5 hardware.

use std::net::UdpSocket;
use std::sync::mpsc;
use std::time::Duration;

use kestrel_core::account::AccountId;
use kestrel_core::discovery::{Discovery, HostState};
use kestrel_core::nal::Codec;
use kestrel_core::secure::Secret;
use kestrel_core::session::{
    Resolution, Session, SessionConfig, SessionEvent, SessionSink, VideoFrame,
};

/// A UDP responder that answers discovery probes the way a PS5 does.
#[test]
fn discovery_parses_ps5_reply_from_loopback_responder() {
    let sock = match UdpSocket::bind("127.0.0.1:9302") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("skipping: cannot bind 127.0.0.1:9302 ({e})");
            return;
        }
    };
    sock.set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let responder = std::thread::spawn(move || {
        let mut buf = [0u8; 512];
        let (n, from) = sock.recv_from(&mut buf).expect("probe received");
        let probe = String::from_utf8_lossy(&buf[..n]).to_string();
        let reply = "HTTP/1.1 200 Ok\n\
                     host-id:AABBCCDDEEFF\n\
                     host-type:PS5\n\
                     host-name:LoopbackConsole\n\
                     host-request-port:997\n\
                     device-discovery-protocol-version:00030010\n\
                     system-version:09000000\n\
                     running-app-titleid:PPSA01234\n\
                     running-app-name:Test Game\n";
        sock.send_to(reply.as_bytes(), from).unwrap();
        probe
    });

    let (tx, rx) = mpsc::channel();
    let disco = Discovery::start(Some("127.0.0.1"), 200, move |hosts| {
        let _ = tx.send(hosts);
    })
    .expect("discovery starts");

    let probe = responder.join().unwrap();
    assert!(
        probe.starts_with("SRCH * HTTP/1.1"),
        "unexpected probe: {probe}"
    );
    assert!(
        probe.contains("00030010"),
        "PS5 protocol version must be advertised"
    );

    let hosts = loop {
        let hosts = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("discovery callback");
        if !hosts.is_empty() {
            break hosts;
        }
    };
    drop(disco);
    let h = &hosts[0];
    assert_eq!(h.name, "LoopbackConsole");
    assert!(h.is_ps5);
    assert_eq!(h.state, HostState::Ready);
    assert_eq!(h.host_id, "AABBCCDDEEFF");
    assert_eq!(h.running_app_name, "Test Game");
    assert_eq!(h.address, "127.0.0.1");
}

struct ChannelSink(mpsc::Sender<SessionEvent>);
impl SessionSink for ChannelSink {
    fn on_event(&self, e: SessionEvent) {
        let _ = self.0.send(e);
    }
    fn on_video(&self, _: VideoFrame<'_>) -> bool {
        true
    }
}

/// With nothing listening, the session must fail cleanly with a Quit event,
/// and dropping it must join libchiaki's threads without hanging.
#[test]
fn session_reports_quit_when_console_unreachable() {
    let (tx, rx) = mpsc::channel();
    let cfg = SessionConfig {
        host: "127.0.0.1".into(),
        ps5: true,
        resolution: Resolution::P720,
        fps: 60,
        bitrate_kbps: None,
        codec: Codec::H264,
        dualsense: false,
        packet_loss_max: 0.05,
        regist_key: Secret::new(*b"00000000\0\0\0\0\0\0\0\0"),
        morning: Secret::new([0; 16]),
        account: AccountId([0; 8]),
    };
    let session = Session::start(cfg, ChannelSink(tx)).expect("session starts");
    let quit = loop {
        match rx
            .recv_timeout(Duration::from_secs(30))
            .expect("quit event")
        {
            q @ SessionEvent::Quit { .. } => break q,
            _ => continue,
        }
    };
    match quit {
        SessionEvent::Quit {
            is_error, message, ..
        } => {
            assert!(is_error);
            assert!(!message.is_empty());
        }
        _ => unreachable!(),
    }
    let stats = session.stats();
    assert_eq!(
        stats.frames_total, 0,
        "no video can arrive without a console"
    );
    drop(session);
}
