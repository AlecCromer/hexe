//! hexe desktop app entry (the Tauri shell lives in `lib.rs`).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
fn main() {
    hexe_tauri_lib::run();
}
