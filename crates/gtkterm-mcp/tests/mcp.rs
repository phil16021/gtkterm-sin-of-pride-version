//! End-to-end MCP tests: a real rmcp client talks to the server over in-memory and HTTP transports.
#![allow(clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use gtkterm_core::config::Config;
use gtkterm_core::manager::SessionManager;
use gtkterm_core::port::loopback::{self, DeviceEnd};
use gtkterm_core::testing::PtyPair;
use gtkterm_mcp::transport::{HttpSecurity, serve_http};
use gtkterm_mcp::{GtkTermMcp, ServerOptions, WRITE_TOOLS};
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ClientConfig, GetPromptRequestParams, ReadResourceRequestParams,
};
use rmcp::service::RunningService;
use rmcp::{ClientHandler, RoleClient, ServiceExt};
use serde_json::{Value, json};

#[derive(Debug, Clone, Default)]
struct Client {
    progress: Arc<std::sync::Mutex<Vec<rmcp::model::ProgressNotificationParam>>>,
}

impl ClientHandler for Client {
    fn get_info(&self) -> ClientConfig {
        ClientConfig::default()
    }

    async fn on_progress(
        &self,
        params: rmcp::model::ProgressNotificationParam,
        _context: rmcp::service::NotificationContext<RoleClient>,
    ) {
        self.progress.lock().unwrap().push(params);
    }
}

type Peer = RunningService<RoleClient, Client>;

struct Fixture {
    client: Peer,
    manager: Arc<SessionManager>,
    _server: tokio::task::JoinHandle<()>,
}

async fn fixture(options: ServerOptions) -> Fixture {
    let manager = SessionManager::new(Config::default(), None);
    let server = GtkTermMcp::new(Arc::clone(&manager), options);
    let (server_io, client_io) = tokio::io::duplex(1 << 16);
    let task = tokio::spawn(async move {
        if let Ok(running) = server.serve(server_io).await {
            let _ = running.waiting().await;
        }
    });
    let client = Client::default()
        .serve(client_io)
        .await
        .expect("client handshake");
    Fixture {
        client,
        manager,
        _server: task,
    }
}

async fn with_device(f: &Fixture) -> DeviceEnd {
    let session = f.manager.first_or_create();
    let (port, dev) = loopback::pair("loopback");
    session.attach(port).await.unwrap();
    dev
}

async fn call(client: &Peer, name: &str, args: Value) -> CallToolResult {
    let params = CallToolRequestParams::new(name.to_owned())
        .with_arguments(args.as_object().cloned().unwrap_or_default());
    client
        .call_tool(params)
        .await
        .unwrap_or_else(|e| panic!("{name}: protocol error {e}"))
}

async fn ok(client: &Peer, name: &str, args: Value) -> Value {
    let r = call(client, name, args).await;
    assert_ne!(r.is_error, Some(true), "{name} failed: {:?}", r.content);
    r.structured_content
        .unwrap_or_else(|| panic!("{name}: no structured content"))
}

async fn tool_error(client: &Peer, name: &str, args: Value) -> String {
    let r = call(client, name, args).await;
    assert_eq!(r.is_error, Some(true), "{name} should fail");
    serde_json::to_string(&r.content).unwrap()
}

#[tokio::test]
async fn lists_tools_with_schemas() {
    let f = fixture(ServerOptions::default()).await;
    let tools = f.client.list_all_tools().await.unwrap();
    let mut names: Vec<_> = tools.iter().map(|t| t.name.to_string()).collect();
    names.sort();
    insta::assert_yaml_snapshot!(names);
    let send = tools.iter().find(|t| t.name == "send").unwrap();
    let schema = serde_json::to_string(&send.input_schema).unwrap();
    assert!(
        schema.contains("line_ending") && schema.contains("base64"),
        "{schema}"
    );
    let read = tools.iter().find(|t| t.name == "read").unwrap();
    assert_eq!(
        read.annotations.as_ref().and_then(|a| a.read_only_hint),
        Some(true)
    );
}

#[tokio::test]
async fn read_only_mode_hides_and_refuses_write_tools() {
    let f = fixture(ServerOptions {
        read_only: true,
        ..ServerOptions::default()
    })
    .await;
    let names: Vec<_> = f
        .client
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    for w in WRITE_TOOLS {
        assert!(!names.iter().any(|n| n == w), "{w} visible in read-only mode");
    }
    assert!(names.iter().any(|n| n == "read"));
    let params =
        CallToolRequestParams::new("send").with_arguments(json!({"data": "x"}).as_object().cloned().unwrap());
    assert!(
        f.client.call_tool(params).await.is_err(),
        "hidden tool must not be callable"
    );
}

#[tokio::test]
async fn send_read_and_expect_against_a_device() {
    let f = fixture(ServerOptions::default()).await;
    let mut dev = with_device(&f).await;

    let sent = ok(&f.client, "send", json!({"data": "AT", "line_ending": "cr"})).await;
    assert_eq!(sent["bytes"], 3);
    assert_eq!(dev.recv().await.unwrap(), &b"AT\r"[..]);
    let cursor = sent["rx_cursor"].as_u64().unwrap();

    dev.send(&b"\r\nOK\r\n"[..]).await.unwrap();
    let r = ok(&f.client, "read", json!({"cursor": cursor, "timeout_ms": 2000})).await;
    assert_eq!(r["data"], "\r\nOK\r\n");
    let next = r["next_cursor"].as_u64().unwrap();
    assert_eq!(next, cursor + 6);

    // Device answers asynchronously after the request.
    let responder = tokio::spawn(async move {
        let req = dev.recv().await.unwrap();
        assert_eq!(req, &b"ATI\r"[..]);
        tokio::time::sleep(Duration::from_millis(30)).await;
        dev.send(&b"Model ESP-12F v2\r\nOK\r\n"[..]).await.unwrap();
        dev
    });
    let m = ok(
        &f.client,
        "expect",
        json!({"send": "ATI", "line_ending": "cr", "pattern": "Model (\\S+) v(\\d+)\\r\\nOK", "timeout_ms": 3000}),
    )
    .await;
    assert_eq!(m["captures"], json!(["ESP-12F", "2"]));
    let dev = responder.await.unwrap();

    let msg = tool_error(&f.client, "expect", json!({"pattern": "NEVER", "timeout_ms": 50})).await;
    assert!(msg.contains("timed out"), "{msg}");

    let hex = ok(
        &f.client,
        "send",
        json!({"data": "01 03 00 00 00 0A C5 CD", "encoding": "hex"}),
    )
    .await;
    assert_eq!(hex["bytes"], 8);
    let mut dev = dev;
    assert_eq!(
        dev.recv().await.unwrap(),
        &[0x01, 0x03, 0, 0, 0, 0x0A, 0xC5, 0xCD][..]
    );

    let found = ok(&f.client, "search_buffer", json!({"pattern": "Model \\S+"})).await;
    assert_eq!(found["matches"][0]["text"], "Model ESP-12F");
    assert_eq!(found["matches"][0]["line"], "Model ESP-12F v2");
}

#[tokio::test]
async fn sunshine_series_are_visible_to_agents() {
    let f = fixture(ServerOptions::default()).await;
    let dev = with_device(&f).await;
    for i in 0..500 {
        let v = f64::from(i) * 0.5;
        dev.send(format!("{{env|Temp|255,0,0|{v}|Hum|{}}}\n", 40 + i % 3).into_bytes())
            .await
            .unwrap();
    }
    dev.send(&b"{B:levels|a|1|b|2}\n"[..]).await.unwrap();
    // Wait until the last packet was processed.
    let mut charts = Value::Null;
    for _ in 0..100 {
        charts = ok(&f.client, "sunshine_list_series", json!({})).await;
        if charts["charts"].as_array().is_some_and(|c| c.len() == 2) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let env = &charts["charts"][0];
    assert_eq!(env["id"], "env");
    assert_eq!(env["kind"], "line");
    assert_eq!(env["series"][0]["max"], 249.5);
    assert_eq!(env["series"][0]["color"], "#FF0000");
    assert_eq!(charts["charts"][1]["kind"], "bar");

    let s = ok(
        &f.client,
        "sunshine_get_series",
        json!({"id": "env", "series": ["Temp"], "max_points": 50}),
    )
    .await;
    let pts = &s["series"][0];
    assert_eq!(pts["available"], 500);
    assert_eq!(pts["values"].as_array().unwrap().len(), 50);
    assert_eq!(pts["values"][49], json!([249.5]));
    let img = call(
        &f.client,
        "sunshine_export_png",
        json!({"id": "env", "width": 400, "height": 300}),
    )
    .await;
    if cfg!(feature = "png") {
        assert_ne!(img.is_error, Some(true), "{:?}", img.content);
        let json = serde_json::to_value(&img.content).unwrap();
        assert_eq!(json[0]["type"], "image");
        assert_eq!(json[0]["mimeType"], "image/png");
    } else {
        assert_eq!(img.is_error, Some(true));
    }

    ok(
        &f.client,
        "sunshine_set_chart_type",
        json!({"id": "env", "kind": "scatter"}),
    )
    .await;
    let csv = ok(&f.client, "sunshine_export_csv", json!({"id": "levels"})).await;
    assert!(
        csv["text"]
            .as_str()
            .unwrap()
            .starts_with("timestamp,elapsed_s,series,index,values\n")
    );
    let msg = tool_error(&f.client, "sunshine_get_series", json!({"id": "nope"})).await;
    assert!(msg.contains("available"), "{msg}");
}

#[tokio::test]
async fn open_serial_port_by_tool_and_log() {
    let pty = PtyPair::new().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let f = fixture(ServerOptions {
        log_dir: dir.path().to_owned(),
        ..ServerOptions::default()
    })
    .await;
    let opened = ok(
        &f.client,
        "open_port",
        json!({"connection": {"type": "serial", "port": pty.path_str(), "baudrate": 921_600}}),
    )
    .await;
    assert_eq!(opened["status"]["state"]["state"], "open");
    assert!(
        opened["status"]["description"]
            .as_str()
            .unwrap()
            .contains("921600-8-N-1")
    );

    let log = ok(
        &f.client,
        "log_start",
        json!({"format": "json", "file_name": "agent.json"}),
    )
    .await;
    assert_eq!(log["format"], "json");
    pty.write(b"hello\n").await.unwrap();
    ok(&f.client, "read", json!({"cursor": 0, "timeout_ms": 2000})).await;
    let stopped = ok(&f.client, "log_stop", json!({})).await;
    assert_eq!(stopped["bytes"], 6);
    let doc: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.path().join("agent.json")).unwrap()).unwrap();
    assert_eq!(doc["data"][0]["hex"], json!([104, 101, 108, 108, 111, 10]));

    let msg = tool_error(&f.client, "log_start", json!({"file_name": "../escape.log"})).await;
    assert!(msg.contains("plain file name"), "{msg}");

    ok(&f.client, "close_port", json!({})).await;
    let status = ok(&f.client, "get_status", json!({})).await;
    assert_eq!(status["state"]["state"], "closed");
    let msg = tool_error(&f.client, "send", json!({"data": "x"})).await;
    assert!(msg.contains("not open"), "{msg}");
}

#[tokio::test]
async fn resources_and_prompts() {
    let f = fixture(ServerOptions::default()).await;
    let _dev = with_device(&f).await;
    let uris: Vec<_> = f
        .client
        .list_all_resources()
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.uri)
        .collect();
    assert!(uris.contains(&"gtkterm://docs/sunshine-protocol".to_owned()));
    assert!(uris.contains(&"gtkterm://session/1/status".to_owned()));
    let doc = f
        .client
        .read_resource(ReadResourceRequestParams::new("gtkterm://docs/sunshine-protocol"))
        .await
        .unwrap();
    let text = serde_json::to_string(&doc.contents).unwrap();
    assert!(text.contains("Sunshine packet format"));
    let status = f
        .client
        .read_resource(ReadResourceRequestParams::new("gtkterm://session/1/status"))
        .await
        .unwrap();
    assert!(
        serde_json::to_string(&status.contents)
            .unwrap()
            .contains("loopback")
    );
    assert!(
        f.client
            .read_resource(ReadResourceRequestParams::new("gtkterm://nope"))
            .await
            .is_err()
    );

    let prompts: Vec<_> = f
        .client
        .list_all_prompts()
        .await
        .unwrap()
        .into_iter()
        .map(|p| p.name)
        .collect();
    for p in [
        "debug-serial-device",
        "analyze-sunshine-data",
        "modbus-scan",
        "at-modem-check",
        "write-sunshine-firmware-snippet",
    ] {
        assert!(prompts.iter().any(|n| n == p), "missing prompt {p}");
    }
    let p = f
        .client
        .get_prompt(
            GetPromptRequestParams::new("modbus-scan")
                .with_arguments(json!({"focus": "RS485 bus"}).as_object().cloned().unwrap()),
        )
        .await
        .unwrap();
    assert!(serde_json::to_string(&p.messages).unwrap().contains("RS485 bus"));
}

#[tokio::test]
async fn http_endpoint_requires_token_and_checks_origin() {
    let manager = SessionManager::new(Config::default(), None);
    let server = GtkTermMcp::new(manager, ServerOptions::default());
    let security = HttpSecurity {
        token: Some("s3cret".into()),
        ..HttpSecurity::default()
    };
    let handle = serve_http(server, "127.0.0.1:0".parse().unwrap(), security)
        .await
        .unwrap();

    let http = reqwest::Client::new();
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#;
    let post = |auth: Option<&str>, origin: Option<&str>| {
        let mut req = http
            .post(&handle.url)
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .header("mcp-protocol-version", "2026-07-28")
            .body(body);
        if let Some(a) = auth {
            req = req.header("authorization", a);
        }
        if let Some(o) = origin {
            req = req.header("origin", o);
        }
        req.send()
    };
    assert_eq!(post(None, None).await.unwrap().status(), 401);
    assert_eq!(post(Some("Bearer wrong"), None).await.unwrap().status(), 401);
    assert_eq!(
        post(Some("Bearer s3cret"), Some("http://evil.example"))
            .await
            .unwrap()
            .status(),
        403
    );

    let transport = rmcp::transport::StreamableHttpClientTransport::from_config(
        rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig::with_uri(
            handle.url.clone(),
        )
        .auth_header("s3cret"),
    );
    let client = Client::default()
        .serve(transport)
        .await
        .expect("authenticated client connects");
    let tools = client.list_all_tools().await.unwrap();
    assert!(tools.iter().any(|t| t.name == "list_ports"));
    let st = client
        .call_tool(CallToolRequestParams::new("get_status").with_arguments(serde_json::Map::new()))
        .await
        .unwrap();
    assert_ne!(st.is_error, Some(true));
    client.cancel().await.unwrap();
    handle.shutdown().await;
}

#[tokio::test]
async fn macros_presets_and_profiles() {
    let pty = PtyPair::new().unwrap();
    let f = fixture(ServerOptions::default()).await;
    let opened = ok(
        &f.client,
        "open_port",
        json!({"connection": {"type": "serial", "port": pty.path_str()}}),
    )
    .await;
    assert!(
        opened["status"]["description"]
            .as_str()
            .unwrap()
            .contains("115200-8-N-1")
    );

    // Macros: save, list, run against a responding device.
    let saved = ok(
        &f.client,
        "macro_save",
        json!({"name": "probe", "text": "send: ATI\\r\nexpect[2000]: OK\\r\\n\n", "shortcut": "F9"}),
    )
    .await;
    assert_eq!(saved["steps"], 2);
    let msg = tool_error(
        &f.client,
        "macro_save",
        json!({"name": "bad", "text": "send: x\nfrobnicate: y\n"}),
    )
    .await;
    assert!(msg.contains("line 2"), "{msg}");
    let listed = ok(&f.client, "macro_list", json!({})).await;
    assert_eq!(listed["macros"][0]["name"], "probe");
    assert_eq!(
        listed["macros"][0]["text"],
        "send: ATI\\r\nexpect[2000]: OK\\r\\n\n"
    );
    let responder = async {
        assert_eq!(pty.read_exact(4).await.unwrap(), b"ATI\r");
        pty.write(b"Modem v1\r\nOK\r\n").await.unwrap();
    };
    let (report, ()) = tokio::join!(ok(&f.client, "macro_run", json!({"name": "probe"})), responder);
    assert_eq!(report["steps"].as_array().unwrap().len(), 2);
    assert!(report["received"].as_str().unwrap().contains("Modem v1"));
    let msg = tool_error(&f.client, "macro_run", json!({"name": "missing"})).await;
    assert!(msg.contains("no macro"), "{msg}");
    let res = f
        .client
        .read_resource(ReadResourceRequestParams::new("gtkterm://macros"))
        .await
        .unwrap();
    assert!(
        serde_json::to_string(&res.contents)
            .unwrap()
            .contains("# probe (F9)")
    );

    // Presets.
    let presets = ok(&f.client, "preset_list", json!({})).await;
    assert_eq!(presets["presets"].as_array().unwrap().len(), 10);
    // Reopening the same device right away (C defect class: lock released late).
    let applied = ok(&f.client, "preset_apply", json!({"name": "modbus rtu"})).await;
    assert!(
        applied["status"]["description"]
            .as_str()
            .unwrap()
            .contains("19200-8-E-1"),
        "{applied}"
    );
    let msg = tool_error(&f.client, "preset_apply", json!({"name": "nope"})).await;
    assert!(msg.contains("preset_list"));

    // Profiles.
    ok(&f.client, "profile_save", json!({"name": "bench"})).await;
    let profiles = ok(&f.client, "profile_list", json!({})).await;
    let names: Vec<_> = profiles["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(names, ["bench", "default"]);
    let bench = ok(&f.client, "profile_get", json!({"name": "bench"})).await;
    assert_eq!(bench["connection"]["baudrate"], 19_200);
    assert_eq!(bench["connection"]["parity"], "even");
}

#[tokio::test]
async fn decoders_and_modbus_requests() {
    use gtkterm_protocols::decode::modbus;

    let f = fixture(ServerOptions::default()).await;
    let mut dev = with_device(&f).await;
    let msg = tool_error(&f.client, "decoder_set", json!({"mode": "bogus"})).await;
    assert!(msg.contains("modbus_rtu"), "{msg}");

    // A fake Modbus slave: unit 7 answers reads of 2 holding registers; unit 9 raises an exception.
    let slave = async {
        let req = dev.recv().await.unwrap();
        assert_eq!(&req[..], modbus::rtu_frame(7, &[0x03, 0x00, 0x10, 0x00, 0x02]));
        dev.send(modbus::rtu_frame(7, &[0x03, 0x04, 0x00, 0x2A, 0x12, 0x34]))
            .await
            .unwrap();
    };
    let (resp, ()) = tokio::join!(
        ok(
            &f.client,
            "modbus_request",
            json!({"unit": 7, "request": {"function": "read_holding_registers", "address": 16, "count": 2}})
        ),
        slave
    );
    assert_eq!(
        resp["response"],
        json!({"kind": "registers", "values": [42, 0x1234]}),
        "{resp}"
    );
    assert_eq!(resp["framing"], "rtu");
    assert_eq!(resp["decoder_changed"], true);
    assert_eq!(resp["request"], "07 03 00 10 00 02 C5 A8");

    let slave = async {
        dev.recv().await.unwrap();
        dev.send(modbus::rtu_frame(9, &[0x86, 0x02])).await.unwrap();
    };
    let (resp, ()) = tokio::join!(
        ok(
            &f.client,
            "modbus_request",
            json!({"unit": 9, "request": {"function": "write_single_register", "address": 1, "value": 5}})
        ),
        slave
    );
    assert_eq!(resp["response"]["kind"], "exception");
    assert_eq!(resp["response"]["name"], "Illegal Data Address");
    assert_eq!(resp["decoder_changed"], false);

    let msg = tool_error(
        &f.client,
        "modbus_request",
        json!({"unit": 3, "timeout_ms": 100, "request": {"function": "read_coils", "address": 0, "count": 8}}),
    )
    .await;
    assert!(msg.contains("no response from unit 3"), "{msg}");
    dev.drain();

    // All requests and responses are in the frame ring.
    let frames = ok(&f.client, "decoder_get_frames", json!({"since": 0})).await;
    let list = frames["frames"].as_array().unwrap();
    let sources: Vec<_> = list.iter().map(|x| x["source"].as_str().unwrap()).collect();
    assert_eq!(sources, ["tx", "rx", "tx", "rx", "tx"], "{frames}");
    assert_eq!(list[1]["check"]["status"], "ok");
    assert_eq!(list[1]["fields"]["slave"], "7");
    assert_eq!(frames["decoder"]["protocol"], "modbus_rtu");
    let next = frames["next_seq"].as_u64().unwrap();

    // Switch to NMEA, wait for a frame that arrives later.
    let st = ok(&f.client, "decoder_set", json!({"mode": "nmea", "clear": true})).await;
    assert_eq!(st["frames"], 0);
    let feeder = async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        dev.send(&b"$GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,*47\r\n"[..])
            .await
            .unwrap();
    };
    let (frames, ()) = tokio::join!(
        ok(
            &f.client,
            "decoder_get_frames",
            json!({"since": next, "timeout_ms": 5000, "include_bytes": false})
        ),
        feeder
    );
    let list = frames["frames"].as_array().unwrap();
    assert_eq!(list.len(), 1, "{frames}");
    assert!(list[0]["summary"].as_str().unwrap().starts_with("[NMEA] GPS GGA"));
    assert!(list[0].get("bytes").is_none());

    let res = f
        .client
        .read_resource(ReadResourceRequestParams::new("gtkterm://session/1/frames"))
        .await
        .unwrap();
    assert!(serde_json::to_string(&res.contents).unwrap().contains("GPS GGA"));
}

/// Run an XMODEM/YMODEM machine as the device on the far end of a loopback port.
async fn device_transfer(
    dev: &mut DeviceEnd,
    mut m: impl gtkterm_protocols::xmodem::Transfer,
) -> Vec<gtkterm_protocols::xmodem::Event> {
    let t0 = std::time::Instant::now();
    let now = || u64::try_from(t0.elapsed().as_millis()).unwrap();
    let mut events = Vec::new();
    let mut out = m.start(now());
    loop {
        events.append(&mut out.events);
        if !out.bytes.is_empty() {
            dev.send(std::mem::take(&mut out.bytes)).await.unwrap();
            out = m.drained(now());
            continue;
        }
        if m.is_done() {
            return events;
        }
        let wait = m
            .deadline()
            .map_or(200, |d| d.saturating_sub(now()).clamp(1, 200));
        out = match tokio::time::timeout(Duration::from_millis(wait), dev.recv()).await {
            Ok(Some(b)) => m.input(&b, now()),
            Ok(None) => return events,
            Err(_) => m.tick(now()),
        };
    }
}

#[tokio::test]
async fn file_transfers_with_progress_and_sandbox() {
    use gtkterm_protocols::xmodem::{Event, FileData, Receiver, Sender, Timing, Variant};

    let transfers = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let f = fixture(ServerOptions {
        transfer_dir: transfers.path().to_owned(),
        ..ServerOptions::default()
    })
    .await;
    let mut dev = with_device(&f).await;
    let data: Vec<u8> = (0..5000u32).map(|i| (i * 13) as u8).collect();
    std::fs::write(transfers.path().join("fw.bin"), &data).unwrap();
    std::fs::write(outside.path().join("secret.txt"), b"no").unwrap();

    // Sandbox: files outside the transfer directory, also via a symlink, are refused.
    let path = outside.path().join("secret.txt");
    let msg = tool_error(
        &f.client,
        "file_send",
        json!({"protocol": "xmodem", "paths": [path]}),
    )
    .await;
    assert!(msg.contains("outside the transfer directory"), "{msg}");
    std::os::unix::fs::symlink(&path, transfers.path().join("link.txt")).unwrap();
    let msg = tool_error(
        &f.client,
        "file_send",
        json!({"protocol": "xmodem", "paths": ["link.txt"]}),
    )
    .await;
    assert!(msg.contains("outside"), "{msg}");
    let msg = tool_error(
        &f.client,
        "file_receive",
        json!({"protocol": "ymodem", "subdir": "../x"}),
    )
    .await;
    assert!(msg.contains("without `..`"), "{msg}");

    // Upload with YMODEM; the device runs a native receiver. Progress arrives as notifications.
    let mut params = CallToolRequestParams::new("file_send").with_arguments(
        json!({"protocol": "ymodem", "paths": ["fw.bin"]})
            .as_object()
            .cloned()
            .unwrap(),
    );
    params.meta = Some(rmcp::model::RequestMetaObject::with_progress_token(
        rmcp::model::ProgressToken(rmcp::model::NumberOrString::String("up".into())),
    ));
    let device = device_transfer(&mut dev, Receiver::new(Variant::Ymodem, Timing::default(), "x"));
    let (result, events) = tokio::join!(f.client.call_tool(params), device);
    let result = result.unwrap();
    assert_ne!(result.is_error, Some(true), "{:?}", result.content);
    let sc = result.structured_content.unwrap();
    assert_eq!(sc["bytes"], 5000, "{sc}");
    assert_eq!(sc["files"][0]["name"], "fw.bin");
    let got = events.iter().find_map(|e| match e {
        Event::FileReceived(f) => Some(f.clone()),
        _ => None,
    });
    assert_eq!(
        got.map(|f| (f.name, f.data)),
        Some(("fw.bin".to_owned(), data.clone()))
    );
    let progress = f.client.service().progress.lock().unwrap().clone();
    assert!(!progress.is_empty(), "progress notifications");
    assert!(progress.iter().all(|p| p.total == Some(5000.0)));
    let last = progress.last().unwrap().message.clone().unwrap_or_default();
    assert!(last.contains("upload finished"), "{last}");
    assert_eq!(progress.last().unwrap().progress, 5000.0);

    // Download with XMODEM into a subdirectory; the device sends.
    let sender = Sender::new(
        Variant::Xmodem,
        vec![FileData {
            name: String::new(),
            data: b"log line\n".to_vec(),
            mtime: None,
        }],
        Timing::default(),
    )
    .unwrap();
    let (r, _) = tokio::join!(
        ok(
            &f.client,
            "file_receive",
            json!({"protocol": "xmodem", "subdir": "logs", "name": "dev.log"})
        ),
        device_transfer(&mut dev, sender)
    );
    let saved = r["files"][0]["path"].as_str().unwrap().to_owned();
    assert_eq!(
        std::path::Path::new(&saved),
        transfers.path().join("logs/dev.log")
    );
    assert_eq!(std::fs::read(&saved).unwrap(), b"log line\n");

    // Nothing to cancel; status shows the last transfer.
    let done = ok(&f.client, "file_cancel", json!({})).await;
    assert_eq!(done["message"], "no transfer was running");
    assert_eq!(done["status"]["transfer"]["state"], "finished");
}

#[tokio::test]
async fn scripts_signals_and_port_control() {
    use gtkterm_core::port::loopback::InputLines;

    let f = fixture(ServerOptions::default()).await;
    let session = f.manager.first_or_create();
    let (port, mut dev) = loopback::pair_with_lines("loop");
    session.attach(port).await.unwrap();

    // Check first: syntax errors carry the line.
    let msg = tool_error(&f.client, "script_check", json!({"script": "send a\nbogus\n"})).await;
    assert!(msg.contains("<script>:2: unknown command"), "{msg}");
    let ok_check = ok(&f.client, "script_check", json!({"script": "exec ls\n"})).await;
    assert_eq!(ok_check["uses_exec"], true);
    assert_eq!(ok_check["exec_allowed"], false);
    let msg = tool_error(&f.client, "script_run", json!({"script": "exec ls\n"})).await;
    assert!(msg.contains("exec is disabled"), "{msg}");

    // A script talking to the device, with variables and logs.
    let device = async {
        let got = dev.recv().await.unwrap();
        assert_eq!(&got[..], b"ID?\r");
        dev.send(&b"ID=4711\r\n"[..]).await.unwrap();
    };
    let script = "set_dtr 1\nsend ID?\\r\nexpect ID=(\\d+)\nlog device ${MATCH_1} for ${who}\n";
    let (report, ()) = tokio::join!(
        ok(
            &f.client,
            "script_run",
            json!({"script": script, "vars": {"who": "agent"}})
        ),
        device
    );
    assert_eq!(report["log"], json!(["device 4711 for agent"]));
    assert_eq!(report["vars"]["MATCH_1"], "4711");

    // Failures name the line and include what was received.
    dev.send(&b"noise\r\n"[..]).await.unwrap();
    let msg = tool_error(
        &f.client,
        "script_run",
        json!({"script": "log start\nexpect[100] never\n"}),
    )
    .await;
    assert!(msg.contains("<script>:2: timed out after 100 ms"), "{msg}");
    assert!(msg.contains("start"), "{msg}");

    // Signals: DTR from the script, CTS from the device.
    dev.set_inputs(InputLines {
        cts: true,
        ..InputLines::default()
    });
    tokio::time::sleep(Duration::from_millis(150)).await;
    let h = ok(&f.client, "signal_history", json!({"seconds": 30})).await;
    let changed: Vec<String> = h["samples"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|s| {
            s["changed"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| c.as_str().unwrap().to_owned())
        })
        .collect();
    assert!(
        changed.contains(&"DTR".to_owned()) && changed.contains(&"CTS".to_owned()),
        "{h}"
    );
    let csv = ok(&f.client, "signal_history", json!({"csv": true})).await;
    assert!(
        csv["csv"]
            .as_str()
            .unwrap()
            .starts_with("Timestamp,DTR,RTS,CTS,DSR,DCD,RI,Changed Signals")
    );

    ok(&f.client, "port_flush", json!({})).await;
    let msg = tool_error(&f.client, "port_auto_baud", json!({})).await;
    assert!(msg.contains("serial connection"), "{msg}");
    let done = ok(&f.client, "script_stop", json!({})).await;
    assert_eq!(done["message"], "nothing was running");
    let doc = f
        .client
        .read_resource(ReadResourceRequestParams::new("gtkterm://docs/script-language"))
        .await
        .unwrap();
    assert!(serde_json::to_string(&doc.contents).unwrap().contains("if_match"));
}

#[tokio::test]
async fn write_tools_ask_the_approver_and_every_call_is_audited() {
    use gtkterm_mcp::{Approval, Approver};
    use std::sync::Mutex;
    let dir = tempfile::tempdir().unwrap();
    let audit = dir.path().join("audit.jsonl");
    let asked: Arc<Mutex<Vec<String>>> = Arc::default();
    let answers = Arc::new(Mutex::new(vec![Approval::Deny, Approval::Once, Approval::Always]));
    let (a, q) = (Arc::clone(&answers), Arc::clone(&asked));
    let f = fixture(ServerOptions {
        audit: Some(audit.clone()),
        approver: Some(Approver::new(move |req| {
            q.lock().unwrap().push(format!("{} {}", req.tool, req.arguments));
            let answer = a.lock().unwrap().remove(0);
            async move { answer }
        })),
        ..ServerOptions::default()
    })
    .await;
    let mut dev = with_device(&f).await;

    // Read tools never ask.
    ok(&f.client, "get_status", json!({})).await;
    // Denied: nothing is sent.
    let msg = tool_error(&f.client, "send", json!({"data": "one"})).await;
    assert!(msg.contains("denied"), "{msg}");
    // Once: this call runs, the next one asks again.
    ok(&f.client, "send", json!({"data": "two"})).await;
    assert_eq!(&dev.recv().await.unwrap()[..], b"two");
    // Always: no more questions.
    ok(&f.client, "send", json!({"data": "three"})).await;
    ok(&f.client, "send", json!({"data": "four"})).await;
    assert_eq!(&dev.recv().await.unwrap()[..], b"three");
    assert_eq!(&dev.recv().await.unwrap()[..], b"four");
    assert_eq!(
        *asked.lock().unwrap(),
        [
            r#"send {"data":"one"}"#,
            r#"send {"data":"two"}"#,
            r#"send {"data":"three"}"#
        ]
    );

    let lines: Vec<Value> = std::fs::read_to_string(&audit)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let summary: Vec<(String, String)> = lines
        .iter()
        .map(|l| {
            (
                l["tool"].as_str().unwrap().to_owned(),
                l["outcome"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let expect = |t: &str, o: &str| (t.to_owned(), o.to_owned());
    assert_eq!(
        summary,
        [
            expect("get_status", "ok"),
            expect("send", "denied"),
            expect("send", "ok"),
            expect("send", "ok"),
            expect("send", "ok"),
        ]
    );
    assert_eq!(lines[2]["arguments"]["data"], "two");
    assert!(lines.iter().all(|l| l["time"].is_string() && l["ms"].is_u64()));
    use std::os::unix::fs::PermissionsExt as _;
    assert_eq!(
        std::fs::metadata(&audit).unwrap().permissions().mode() & 0o777,
        0o600
    );
    // Failing tools are recorded with their message.
    tool_error(&f.client, "sunshine_get_series", json!({"id": "nope"})).await;
    let last: Value =
        serde_json::from_str(std::fs::read_to_string(&audit).unwrap().lines().last().unwrap()).unwrap();
    assert_eq!(last["outcome"], "error");
    assert!(last["error"].as_str().unwrap().contains("nope"), "{last}");
}
