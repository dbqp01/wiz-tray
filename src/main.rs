#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(windows)]
mod windows_app;

#[cfg(windows)]
fn main() {
    if let Err(error) = windows_app::run() {
        windows_app::show_error(&error);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!(
        "WiZ Tray es una app de bandeja para Windows; use cargo test para verificar el protocolo."
    );
}
