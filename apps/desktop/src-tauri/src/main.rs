//! Desktop GUI entry point.
//!
//! All behaviour lives in the library crate (`service_orchestrator_desktop_lib`)
//! so the adapter logic stays testable without a window; this binary only
//! starts the Tauri event loop.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    service_orchestrator_desktop_lib::run()
}
