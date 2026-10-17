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
struct Client;

impl ClientHandler for Client {
    fn get_info(&self) -> ClientConfig {
        ClientConfig::default()
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
    let client = Client.serve(client_io).await.expect("client handshake");
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
    let client = Client
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
    let opened = ok(&f.client, "open_port", json!({"connection": {"type": "serial", "port": pty.path_str()}})).await;
    assert!(opened["status"]["description"].as_str().unwrap().contains("115200-8-N-1"));

    // Macros: save, list, run against a responding device.
    let saved = ok(&f.client, "macro_save", json!({"name": "probe", "text": "send: ATI\\r\nexpect[2000]: OK\\r\\n\n", "shortcut": "F9"})).await;
    assert_eq!(saved["steps"], 2);
    let msg = tool_error(&f.client, "macro_save", json!({"name": "bad", "text": "send: x\nfrobnicate: y\n"})).await;
    assert!(msg.contains("line 2"), "{msg}");
    let listed = ok(&f.client, "macro_list", json!({})).await;
    assert_eq!(listed["macros"][0]["name"], "probe");
    assert_eq!(listed["macros"][0]["text"], "send: ATI\\r\nexpect[2000]: OK\\r\\n\n");
    let responder = async {
        assert_eq!(pty.read_exact(4).await.unwrap(), b"ATI\r");
        pty.write(b"Modem v1\r\nOK\r\n").await.unwrap();
    };
    let (report, ()) = tokio::join!(ok(&f.client, "macro_run", json!({"name": "probe"})), responder);
    assert_eq!(report["steps"].as_array().unwrap().len(), 2);
    assert!(report["received"].as_str().unwrap().contains("Modem v1"));
    let msg = tool_error(&f.client, "macro_run", json!({"name": "missing"})).await;
    assert!(msg.contains("no macro"), "{msg}");
    let res = f.client.read_resource(ReadResourceRequestParams::new("gtkterm://macros")).await.unwrap();
    assert!(serde_json::to_string(&res.contents).unwrap().contains("# probe (F9)"));

    // Presets.
    let presets = ok(&f.client, "preset_list", json!({})).await;
    assert_eq!(presets["presets"].as_array().unwrap().len(), 10);
    let applied = ok(&f.client, "preset_apply", json!({"name": "modbus rtu"})).await;
    assert!(applied["status"]["description"].as_str().unwrap().contains("19200-8-E-1"), "{applied}");
    let msg = tool_error(&f.client, "preset_apply", json!({"name": "nope"})).await;
    assert!(msg.contains("preset_list"));

    // Profiles.
    ok(&f.client, "profile_save", json!({"name": "bench"})).await;
    let profiles = ok(&f.client, "profile_list", json!({})).await;
    let names: Vec<_> = profiles["profiles"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap().to_owned()).collect();
    assert_eq!(names, ["bench", "default"]);
    let bench = ok(&f.client, "profile_get", json!({"name": "bench"})).await;
    assert_eq!(bench["connection"]["baudrate"], 19_200);
    assert_eq!(bench["connection"]["parity"], "even");
}
