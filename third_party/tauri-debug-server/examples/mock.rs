//! Display-less server on a MockRuntime app, for exercising app-tools scripts: `cargo run -p tauri-debug-server --example mock -- 9399`.
//! /info /json /console work; /eval times out (a mock window has no JS engine).
fn main() {
    let port: u16 = std::env::args().nth(1).and_then(|p| p.parse().ok()).unwrap_or(9399);
    let app =
        tauri::test::mock_builder().build(tauri::test::mock_context(tauri::test::noop_assets())).expect("mock app");
    tauri::WebviewWindowBuilder::new(&app, "main", tauri::WebviewUrl::default()).build().expect("mock window");
    tauri_debug_server::spawn(app.handle().clone(), port);
    std::thread::sleep(std::time::Duration::from_secs(60));
}
