//! Build script for the Tauri desktop adapter.
//!
//! [`tauri_build`] validates `tauri.conf.json` and generates the capability
//! artifacts the webview runtime enforces at startup.
fn main() {
    tauri_build::build()
}
