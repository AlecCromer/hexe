//! Live HTTP round-trips against the real server on a tauri MockRuntime app (no window, no display).
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;
use tauri::Manager;

fn free_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn http(port: u16, method: &str, path: &str, body: &str) -> (u16, String) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(15))).unwrap();
    write!(
        s,
        "{method} {path} HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    let status = out.split_whitespace().nth(1).unwrap().parse().unwrap();
    (
        status,
        out.split_once("\r\n\r\n")
            .map(|x| x.1)
            .unwrap_or("")
            .to_string(),
    )
}

fn boot() -> u16 {
    let app = tauri::test::mock_builder()
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    tauri::WebviewWindowBuilder::new(&app, "main", tauri::WebviewUrl::default())
        .build()
        .unwrap();
    let port = free_port();
    tauri_debug_server::spawn(app.handle().clone(), port);
    // keep the app alive for the whole test process
    std::mem::forget(app);
    thread::sleep(Duration::from_millis(200));
    port
}

#[test]
fn server_routes_on_test_app() {
    let port = boot();

    // /info lists the main window
    let (st, body) = http(port, "GET", "/info", "");
    assert_eq!(st, 200, "{body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(v["windows"][0]["label"], "main");

    // /json CDP target list
    let (st, body) = http(port, "GET", "/json", "");
    assert_eq!(st, 200);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v[0]["id"], "main");
    assert_eq!(
        v[0]["webSocketDebuggerUrl"],
        format!("ws://127.0.0.1:{port}/devtools/page/main")
    );

    // console: page posts /__log, lane reads /console, ?clear=1 empties
    let (st, _) = http(
        port,
        "POST",
        "/__log",
        r#"{"ts":1,"level":"warn","msg":"hi"}"#,
    );
    assert_eq!(st, 204);
    let (_, body) = http(port, "GET", "/console?clear=1", "");
    assert!(body.contains("\"msg\":\"hi\""), "{body}");
    let (_, body) = http(port, "GET", "/console", "");
    assert_eq!(body, "[]");

    // /eval: server pushes script into the window, page answers via /__result (we play the page)
    let t = thread::spawn(move || http(port, "POST", "/eval", "1+1"));
    thread::sleep(Duration::from_millis(300));
    let (st, _) = http(port, "POST", "/__result", r#"{"id":1,"ok":true,"value":2}"#);
    assert_eq!(st, 204);
    let (st, body) = t.join().unwrap();
    assert_eq!(st, 200);
    assert_eq!(body, r#"{"id":1,"ok":true,"value":2}"#);

    // unknown route
    assert_eq!(http(port, "GET", "/nope", "").0, 404);
}

#[test]
fn init_script_and_port_env() {
    assert!(tauri_debug_server::init_script(9343).contains("http://127.0.0.1:9343/__log"));
    assert_eq!(tauri_debug_server::init_script(0), "");
    assert_eq!(
        tauri_debug_server::port_from_env("TDS_TEST_UNSET_VAR", 9353),
        9353
    );
}

#[test]
fn deferred_config_windows_get_built_in_setup() {
    let mut ctx = tauri::test::mock_context(tauri::test::noop_assets());
    ctx.config_mut().app.windows =
        vec![serde_json::from_str(r#"{"label":"main","title":"T"}"#).unwrap()];
    let cfgs = tauri_debug_server::defer_windows(ctx.config_mut());
    assert_eq!(cfgs.len(), 1);
    let app = tauri::test::mock_builder().build(ctx).unwrap();
    assert!(
        app.get_webview_window("main").is_none(),
        "create=false must defer"
    );
    tauri_debug_server::build_windows(app.handle(), &cfgs, 9999).unwrap();
    assert!(app.get_webview_window("main").is_some());
}
