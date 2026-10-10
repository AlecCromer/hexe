//! hexe's real tauri.conf.json + real `attach_debug` wiring on a MockRuntime app (no window, no display).
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

fn get(port: u16, path: &str) -> String {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    write!(s, "GET {path} HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    out.split_once("\r\n\r\n").map(|x| x.1).unwrap_or("").to_string()
}

#[test]
fn debug_door_answers_info_for_hexe_main_window() {
    let port = TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port();
    let mut ctx = hexe_tauri_lib::context();
    let windows = tauri_debug_server::defer_windows(ctx.config_mut());
    assert_eq!(windows.len(), 1);
    assert_eq!(windows[0].label, "main");
    assert_eq!(windows[0].title, "Hexe");
    let app = tauri::test::mock_builder().build(ctx).unwrap();
    hexe_tauri_lib::attach_debug(app.handle(), &windows, port).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    let info: serde_json::Value = serde_json::from_str(&get(port, "/info")).unwrap();
    assert_eq!(info["ok"], true);
    assert_eq!(info["windows"][0]["label"], "main");
    let json: serde_json::Value = serde_json::from_str(&get(port, "/json")).unwrap();
    assert_eq!(json[0]["id"], "main");
    assert_eq!(hexe_tauri_lib::debug_port(), 9353);
}
