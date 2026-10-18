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
    /// Name reported in `clientInfo` (default: rmcp's).
    name: Option<&'static str>,
    /// Protocol version to ask for (default: the latest).
    version: Option<rmcp::model::ProtocolVersion>,
    /// Legacy notifications: updated URIs, and "list" for list changes.
    updates: Arc<std::sync::Mutex<Vec<String>>>,
}

impl ClientHandler for Client {
    fn get_info(&self) -> ClientConfig {
        let mut c = ClientConfig::default();
        if let Some(name) = self.name {
            c.client_info = rmcp::model::Implementation::new(name, "1.0");
        }
        if let Some(v) = &self.version {
            c.protocol_version = v.clone();
        }
        c
    }

    async fn on_resource_updated(
        &self,
        params: rmcp::model::ResourceUpdatedNotificationParam,
        _context: rmcp::service::NotificationContext<RoleClient>,
    ) {
        self.updates.lock().unwrap().push(params.uri);
    }

    async fn on_resource_list_changed(&self, _context: rmcp::service::NotificationContext<RoleClient>) {
        self.updates.lock().unwrap().push("list".into());
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
    fixture_with(options, Client::default()).await
}

async fn fixture_with(options: ServerOptions, client: Client) -> Fixture {
    let manager = SessionManager::new(Config::default(), None);
    let server = GtkTermMcp::new(Arc::clone(&manager), options);
    let (server_io, client_io) = tokio::io::duplex(1 << 16);
    let task = tokio::spawn(async move {
        if let Ok(running) = server.serve(server_io).await {
            let _ = running.waiting().await;
        }
    });
    // 2026-07-28 has no initialize handshake; such clients discover the server.
    let client = match client.version.clone() {
        Some(v) if !v.has_initialize() => {
            use rmcp::ClientServiceExt as _;
            client
                .serve_with_lifecycle(
                    client_io,
                    rmcp::ClientLifecycleMode::Discover {
                        preferred_versions: vec![v],
                    },
                )
                .await
        }
        _ => client.serve(client_io).await,
    }
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

    // A script that outlives the call's timeout is stopped and frees the session.
    let msg = tool_error(
        &f.client,
        "script_run",
        json!({"script": "delay 5000\n", "timeout_ms": 100}),
    )
    .await;
    assert!(msg.contains("did not finish within 100 ms"), "{msg}");
    let report = ok(&f.client, "script_run", json!({"script": "log again\n"})).await;
    assert_eq!(report["log"], json!(["again"]));

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
    use std::os::unix::fs::PermissionsExt as _;
    use std::sync::Mutex;

    use gtkterm_mcp::{Approval, Approver};
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

/// "Allow until the server stops" applies to the client that was asked, not to others.
#[tokio::test]
async fn approval_is_remembered_per_client() {
    use std::sync::Mutex;

    use gtkterm_mcp::{Approval, Approver};
    let asked: Arc<Mutex<Vec<String>>> = Arc::default();
    let q = Arc::clone(&asked);
    let manager = SessionManager::new(Config::default(), None);
    let server = GtkTermMcp::new(
        Arc::clone(&manager),
        ServerOptions {
            approver: Some(Approver::new(move |req| {
                q.lock().unwrap().push(req.client.unwrap_or_default());
                async { Approval::Always }
            })),
            ..ServerOptions::default()
        },
    );
    let connect = |name: &'static str| {
        let server = server.clone();
        async move {
            let (server_io, client_io) = tokio::io::duplex(1 << 16);
            tokio::spawn(async move {
                if let Ok(running) = server.serve(server_io).await {
                    let _ = running.waiting().await;
                }
            });
            Client {
                name: Some(name),
                ..Client::default()
            }
            .serve(client_io)
            .await
            .unwrap()
        }
    };
    let a = connect("agent-a").await;
    let b = connect("agent-b").await;
    let session = manager.first_or_create();
    let (port, _dev) = loopback::pair("loopback");
    session.attach(port).await.unwrap();
    ok(&a, "send", json!({"data": "1"})).await;
    ok(&a, "send", json!({"data": "2"})).await;
    ok(&b, "send", json!({"data": "3"})).await;
    ok(&b, "send", json!({"data": "4"})).await;
    assert_eq!(*asked.lock().unwrap(), ["agent-a 1.0", "agent-b 1.0"]);
}

/// The next notification of a subscription (at most 5 s).
async fn next(sub: &mut rmcp::service::Subscription) -> rmcp::model::ServerNotification {
    tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .expect("a notification")
        .unwrap()
        .expect("subscription open")
}

/// Wait until `cond` holds (polling), at most 5 s.
async fn eventually(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !cond() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn both_protocol_versions_are_served() {
    use rmcp::model::ProtocolVersion;
    for version in [ProtocolVersion::V_2026_07_28, ProtocolVersion::V_2025_11_25] {
        let f = fixture_with(
            ServerOptions::default(),
            Client {
                version: Some(version.clone()),
                ..Client::default()
            },
        )
        .await;
        let negotiated = f.client.peer().peer_info().map(|i| i.protocol_version.clone());
        assert_eq!(negotiated, Some(version.clone()), "{version:?}");
        let tools = f.client.list_all_tools().await.unwrap();
        assert!(tools.iter().any(|t| t.name == "send"), "{version:?}");
        let _dev = with_device(&f).await;
        ok(&f.client, "send", json!({"data": "hi"})).await;
    }
}

/// Wrong arguments are tool errors the model can correct (MCP 2025-11-25 and later); an unknown
/// tool is a protocol error.
#[tokio::test]
async fn bad_arguments_and_unknown_tools() {
    let f = fixture(ServerOptions::default()).await;
    let msg = tool_error(&f.client, "send", json!({"data": 42})).await;
    assert!(msg.contains("expected a string"), "{msg}");
    match f
        .client
        .call_tool(CallToolRequestParams::new("no_such_tool"))
        .await
    {
        Err(rmcp::ServiceError::McpError(e)) => assert_eq!(e.code.0, -32602, "{e:?}"),
        other => panic!("expected invalid params, got {other:?}"),
    }
}

#[tokio::test]
async fn listen_reports_updates_and_new_sessions() {
    use rmcp::model::{ServerNotification, SubscriptionFilter};
    let f = fixture_with(
        ServerOptions::default(),
        Client {
            version: Some(rmcp::model::ProtocolVersion::V_2026_07_28),
            ..Client::default()
        },
    )
    .await;
    let dev = with_device(&f).await;
    let filter = SubscriptionFilter::builder()
        .resource_subscriptions(["gtkterm://session/1/rx", "gtkterm://docs/sunshine-protocol"])
        .resources_list_changed()
        .build();
    let mut sub = f.client.peer().listen(filter).await.unwrap();
    assert_eq!(
        sub.acknowledged().resource_subscriptions.as_deref(),
        Some(&["gtkterm://session/1/rx".to_owned()][..]),
        "static documents are not watchable"
    );
    // A burst of data gives one update per throttle period, not one per chunk.
    for i in 0..50 {
        dev.send(format!("line {i}\n").into_bytes()).await.unwrap();
    }
    match next(&mut sub).await {
        ServerNotification::ResourceUpdatedNotification(n) => {
            assert_eq!(n.params.uri, "gtkterm://session/1/rx");
        }
        other => panic!("unexpected {other:?}"),
    }
    let s2 = f
        .manager
        .create("second", gtkterm_core::config::TerminalConfig::default());
    loop {
        match next(&mut sub).await {
            ServerNotification::ResourceListChangedNotification(_) => break,
            ServerNotification::ResourceUpdatedNotification(n) => {
                assert_eq!(n.params.uri, "gtkterm://session/1/rx");
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    // Data in the second session is not subscribed.
    let (port, dev2) = loopback::pair("loop2");
    s2.attach(port).await.unwrap();
    dev2.send(&b"other\n"[..]).await.unwrap();
    // Late updates of session 1 may still arrive; nothing comes for session 2.
    let quiet = tokio::time::Instant::now() + Duration::from_millis(400);
    while let Ok(n) = tokio::time::timeout_at(quiet, sub.next()).await {
        match n.unwrap().expect("subscription open") {
            ServerNotification::ResourceUpdatedNotification(n) => {
                assert_eq!(
                    n.params.uri, "gtkterm://session/1/rx",
                    "nothing for unsubscribed resources"
                );
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}

#[tokio::test]
async fn legacy_subscribe_and_unsubscribe() {
    use rmcp::model::{ProtocolVersion, SubscribeRequestParams, UnsubscribeRequestParams};
    let client = Client {
        version: Some(ProtocolVersion::V_2025_11_25),
        ..Client::default()
    };
    let updates = Arc::clone(&client.updates);
    let f = fixture_with(ServerOptions::default(), client).await;
    let dev = with_device(&f).await;
    #[allow(deprecated)]
    {
        let bad = f
            .client
            .subscribe(SubscribeRequestParams::new("gtkterm://docs/sunshine-protocol"))
            .await;
        assert!(bad.is_err(), "documents do not change");
        f.client
            .subscribe(SubscribeRequestParams::new("gtkterm://session/1/status"))
            .await
            .unwrap();
    }
    dev.send(&b"x\n"[..]).await.unwrap();
    eventually("a status update", || {
        updates
            .lock()
            .unwrap()
            .iter()
            .any(|u| u == "gtkterm://session/1/status")
    })
    .await;
    #[allow(deprecated)]
    f.client
        .unsubscribe(UnsubscribeRequestParams::new("gtkterm://session/1/status"))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    updates.lock().unwrap().clear();
    dev.send(&b"y\n"[..]).await.unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        updates.lock().unwrap().is_empty(),
        "{:?}",
        updates.lock().unwrap()
    );
}

#[tokio::test]
async fn completions_for_templates_and_prompts() {
    use rmcp::model::{ArgumentInfo, CompleteRequestParams, CompletionContext, Reference};
    let f = fixture(ServerOptions::default()).await;
    let dev = with_device(&f).await;
    dev.send(&b"{imu|Roll|1}\n{env|T|2}\n{imx|A|3}\n"[..])
        .await
        .unwrap();
    eventually("charts", || {
        f.manager.list()[0].with_sunshine(|s| s.charts().len()) == 3
    })
    .await;
    let complete = |r: Reference, name: &str, value: &str, ctx: Option<(&str, &str)>| {
        let mut p = CompleteRequestParams::new(r, ArgumentInfo::new(name, value));
        if let Some((k, v)) = ctx {
            p.context = Some(CompletionContext::with_arguments(
                [(k.to_owned(), v.to_owned())].into(),
            ));
        }
        let client = &f.client;
        async move { client.complete(p).await.unwrap().completion.values }
    };
    let template = || Reference::for_resource("gtkterm://session/{id}/sunshine/{packet_id}");
    assert_eq!(complete(template(), "id", "", None).await, ["1"]);
    assert_eq!(
        complete(template(), "packet_id", "im", Some(("id", "1"))).await,
        ["imu", "imx"]
    );
    assert!(
        complete(template(), "packet_id", "", Some(("id", "9")))
            .await
            .is_empty()
    );
    assert_eq!(
        complete(Reference::for_prompt("analyze-sunshine-data"), "focus", "E", None).await,
        ["env"]
    );
}

#[tokio::test]
async fn write_tools_are_rate_limited_per_client() {
    let f = fixture(ServerOptions {
        max_writes_per_second: 2,
        ..ServerOptions::default()
    })
    .await;
    let _dev = with_device(&f).await;
    for i in 0..4 {
        ok(&f.client, "send", json!({"data": i.to_string()})).await;
    }
    let msg = tool_error(&f.client, "send", json!({"data": "5"})).await;
    assert!(msg.contains("rate limit"), "{msg}");
    // Read tools are not limited.
    ok(&f.client, "get_status", json!({})).await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    ok(&f.client, "send", json!({"data": "6"})).await;
}

#[tokio::test]
async fn activity_stream_reports_calls() {
    let manager = SessionManager::new(Config::default(), None);
    let server = GtkTermMcp::new(Arc::clone(&manager), ServerOptions::default());
    let mut activity = server.activity();
    let (server_io, client_io) = tokio::io::duplex(1 << 16);
    tokio::spawn(async move {
        if let Ok(running) = server.serve(server_io).await {
            let _ = running.waiting().await;
        }
    });
    let client = Client::default().serve(client_io).await.unwrap();
    ok(&client, "get_status", json!({})).await;
    tool_error(&client, "send", json!({"data": "x"})).await;
    let a = activity.recv().await.unwrap();
    assert_eq!((a.tool.as_str(), a.outcome.as_str()), ("get_status", "ok"));
    let b = activity.recv().await.unwrap();
    assert_eq!((b.tool.as_str(), b.outcome.as_str()), ("send", "error"));
    assert!(b.error.unwrap().contains("not open"));
    assert_eq!(b.arguments, r#"{"data":"x"}"#);
}

#[tokio::test]
async fn send_pacing_byte_patterns_and_cancelled_expect() {
    let f = fixture(ServerOptions::default()).await;
    let mut dev = with_device(&f).await;
    let started = std::time::Instant::now();
    ok(&f.client, "send", json!({"data": "abc", "char_delay_ms": 30})).await;
    assert!(
        started.elapsed() >= Duration::from_millis(55),
        "{:?}",
        started.elapsed()
    );
    for want in [b"a", b"b", b"c"] {
        assert_eq!(&dev.recv().await.unwrap()[..], want);
    }
    let msg = tool_error(&f.client, "send", json!({"data": "x", "char_delay_ms": 5000})).await;
    assert!(msg.contains("exceeds 1000"), "{msg}");

    // Literal bytes, including ones that are regex syntax and not UTF-8.
    let cursor = f.manager.list()[0].rx_cursor();
    dev.send(&b"noise.\xAA\x55*end"[..]).await.unwrap();
    let m = ok(
        &f.client,
        "expect",
        json!({"pattern": "AA 55 2A", "pattern_encoding": "hex", "cursor": cursor, "timeout_ms": 2000}),
    )
    .await;
    assert_eq!(m["cursor"].as_u64().unwrap(), cursor + 9);
    let msg = tool_error(
        &f.client,
        "expect",
        json!({"pattern": "", "pattern_encoding": "hex"}),
    )
    .await;
    assert!(msg.contains("empty"), "{msg}");

    // A cancelled expect gives the session back at once.
    let params = CallToolRequestParams::new("expect").with_arguments(
        json!({"pattern": "never", "timeout_ms": 600_000})
            .as_object()
            .cloned()
            .unwrap(),
    );
    let handle = f
        .client
        .peer()
        .send_cancellable_request(
            rmcp::model::ClientRequest::CallToolRequest(rmcp::model::CallToolRequest::new(params)),
            rmcp::service::PeerRequestOptions::no_options(),
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    handle.cancel(Some("test".into())).await.unwrap();
    let started = std::time::Instant::now();
    dev.send(&b"ready\n"[..]).await.unwrap();
    ok(
        &f.client,
        "expect",
        json!({"pattern": "ready", "cursor": 0, "timeout_ms": 2000}),
    )
    .await;
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[tokio::test]
async fn profiles_can_be_deleted() {
    let f = fixture(ServerOptions::default()).await;
    let _dev = with_device(&f).await;
    f.manager.list()[0]
        .open(gtkterm_core::config::Connection::Udp {
            bind: "127.0.0.1".into(),
            local_port: 0,
            remote_host: None,
            remote_port: 0,
            max_datagram: 0,
        })
        .await
        .unwrap();
    ok(&f.client, "profile_save", json!({"name": "bench"})).await;
    let listed = ok(&f.client, "profile_list", json!({})).await;
    assert!(listed.to_string().contains("bench"));
    assert_eq!(
        ok(&f.client, "profile_delete", json!({"name": "bench"})).await["deleted"],
        "bench"
    );
    let listed = ok(&f.client, "profile_list", json!({})).await;
    assert!(!listed.to_string().contains("bench"), "{listed}");
    let msg = tool_error(&f.client, "profile_delete", json!({"name": "bench"})).await;
    assert!(msg.contains("bench"), "{msg}");
}

#[tokio::test]
async fn configure_port_reopens_with_new_settings() {
    let pty = PtyPair::new().unwrap();
    let f = fixture(ServerOptions::default()).await;
    let msg = tool_error(&f.client, "configure_port", json!({"baudrate": 9600})).await;
    assert!(msg.contains("not open"), "{msg}");
    ok(
        &f.client,
        "open_port",
        json!({"connection": {"type": "serial", "port": pty.path_str(), "baudrate": 115_200}}),
    )
    .await;
    pty.write(b"before\n").await.unwrap();
    ok(
        &f.client,
        "expect",
        json!({"pattern": "before", "cursor": 0, "timeout_ms": 2000}),
    )
    .await;
    let done = ok(
        &f.client,
        "configure_port",
        json!({"baudrate": 57_600, "parity": "even", "bits": 7}),
    )
    .await;
    let desc = done["status"]["description"].as_str().unwrap();
    assert!(desc.contains("57600-7-E-1"), "{desc}");
    let msg = tool_error(&f.client, "configure_port", json!({"parity": "sideways"})).await;
    assert!(msg.contains("parity"), "{msg}");
    let rx = ok(&f.client, "read", json!({"cursor": 0})).await;
    assert!(rx["data"].as_str().unwrap().contains("before"), "history is kept");
}

#[tokio::test]
async fn rx_since_and_log_resources_and_series_windows() {
    let dir = tempfile::tempdir().unwrap();
    let f = fixture(ServerOptions {
        log_dir: dir.path().to_owned(),
        ..ServerOptions::default()
    })
    .await;
    let dev = with_device(&f).await;
    let read = |uri: String| {
        let client = &f.client;
        async move {
            let r = client.read_resource(ReadResourceRequestParams::new(uri)).await?;
            let v = serde_json::to_value(&r.contents[0]).unwrap();
            Ok::<String, rmcp::ServiceError>(v["text"].as_str().unwrap().to_owned())
        }
    };
    ok(&f.client, "log_start", json!({"file_name": "s.log"})).await;
    dev.send(&b"hello\nworld\n"[..]).await.unwrap();
    eventually("data", || f.manager.list()[0].rx_cursor() == 12).await;
    let body: Value = serde_json::from_str(
        &read("gtkterm://session/1/rx?since=6&encoding=hex".into())
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["data"], "77 6F 72 6C 64 0A");
    assert_eq!(body["next_cursor"], 12);
    assert!(read("gtkterm://session/1/rx?since=x".into()).await.is_err());
    assert!(read("gtkterm://session/1/status?since=1".into()).await.is_err());
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let log = loop {
        let log: Value =
            serde_json::from_str(&read("gtkterm://session/1/log".into()).await.unwrap()).unwrap();
        if log["tail"].as_str().unwrap().contains("world") {
            break log;
        }
        assert!(tokio::time::Instant::now() < deadline, "log tail: {log}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert!(log["log"]["path"].as_str().unwrap().ends_with("s.log"));

    // Time window: samples from the last 0.3 s of the chart.
    dev.send(&b"{w|v|1}\n{w|v|2}\n"[..]).await.unwrap();
    eventually("two samples", || {
        f.manager.list()[0].with_sunshine(|s| s.chart("w").map_or(0, |c| c.series[0].samples.len())) == 2
    })
    .await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    dev.send(&b"{w|v|3}\n"[..]).await.unwrap();
    eventually("third sample", || {
        f.manager.list()[0].with_sunshine(|s| s.chart("w").map_or(0, |c| c.series[0].samples.len())) == 3
    })
    .await;
    let s = ok(
        &f.client,
        "sunshine_get_series",
        json!({"id": "w", "last_seconds": 0.3}),
    )
    .await;
    assert_eq!(s["series"][0]["values"], json!([[3.0]]));
    let msg = tool_error(
        &f.client,
        "sunshine_get_series",
        json!({"id": "w", "last_seconds": -1}),
    )
    .await;
    assert!(msg.contains("positive"), "{msg}");
}

#[tokio::test]
async fn http_clients_can_listen_for_updates() {
    use rmcp::ClientServiceExt as _;
    use rmcp::model::{ProtocolVersion, ServerNotification, SubscriptionFilter};
    let manager = SessionManager::new(Config::default(), None);
    let session = manager.first_or_create();
    let (port, dev) = loopback::pair("loopback");
    session.attach(port).await.unwrap();
    let server = GtkTermMcp::new(Arc::clone(&manager), ServerOptions::default());
    let handle = serve_http(server, "127.0.0.1:0".parse().unwrap(), HttpSecurity::default())
        .await
        .unwrap();
    let transport = rmcp::transport::StreamableHttpClientTransport::from_config(
        rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig::with_uri(
            handle.url.clone(),
        ),
    );
    let client = Client::default()
        .serve_with_lifecycle(
            transport,
            rmcp::ClientLifecycleMode::Discover {
                preferred_versions: vec![ProtocolVersion::V_2026_07_28],
            },
        )
        .await
        .unwrap();
    let filter = SubscriptionFilter::builder()
        .resource_subscriptions(["gtkterm://session/1/rx"])
        .build();
    let mut sub = client.peer().listen(filter).await.unwrap();
    dev.send(&b"over http\n"[..]).await.unwrap();
    let note = tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .expect("an update")
        .unwrap()
        .expect("subscription open");
    assert!(
        matches!(&note, ServerNotification::ResourceUpdatedNotification(n) if n.params.uri == "gtkterm://session/1/rx"),
        "{note:?}"
    );
    drop(sub);
    client.cancel().await.unwrap();
    handle.shutdown().await;
}

#[tokio::test]
async fn tcp_server_reads_name_the_client() {
    use tokio::io::AsyncWriteExt as _;
    let f = fixture(ServerOptions::default()).await;
    let conn = gtkterm_core::config::Connection::TcpServer {
        bind: "127.0.0.1".into(),
        port: 0,
    };
    let port = gtkterm_core::port::open(&conn).await.unwrap();
    let local: u16 = port.description.rsplit(':').next().unwrap().parse().unwrap();
    let session = f.manager.first_or_create();
    session.attach(port).await.unwrap();
    let mut a = tokio::net::TcpStream::connect(("127.0.0.1", local))
        .await
        .unwrap();
    let mut b = tokio::net::TcpStream::connect(("127.0.0.1", local))
        .await
        .unwrap();
    a.write_all(b"from-a;").await.unwrap();
    eventually("a's data", || session.rx_cursor() == 7).await;
    b.write_all(b"from-b;").await.unwrap();
    eventually("b's data", || session.rx_cursor() == 14).await;
    let r = ok(&f.client, "read", json!({"cursor": 0})).await;
    assert_eq!(r["data"], "from-a;from-b;");
    assert_eq!(
        r["peers"],
        json!([
            {"offset": 0, "bytes": 7, "peer": a.local_addr().unwrap().to_string()},
            {"offset": 7, "bytes": 7, "peer": b.local_addr().unwrap().to_string()},
        ])
    );
}
