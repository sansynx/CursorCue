#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#[cfg(not(windows))]
compile_error!("CursorCue supports Windows only.");
#[cfg(windows)]
mod native;
#[cfg(windows)]
mod presentation;
#[cfg(windows)]
mod settings;
#[cfg(windows)]
fn main() {
    if let Err(error) = native::run() {
        if std::env::args().any(|arg| arg == "--diagnostic") {
            eprintln!("CursorCue diagnostic failed: {error}");
            std::process::exit(1);
        }
        native::report_error(&format!("CursorCue could not start.\n{error}"));
    }
}
