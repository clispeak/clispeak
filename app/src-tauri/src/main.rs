//! Desktop entry point. Mobile enters through `lib.rs` instead.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // The Windows installer and uninstaller call this binary to edit the
    // user's PATH, rather than doing it in NSIS. The reasoning is in
    // `winpath`: NSIS reads a registry string into a 1024-character buffer
    // and would silently truncate — and then write back — a developer's PATH.
    //
    // Handled before Tauri starts, because these runs must not open a window,
    // must not start a node, and must exit with a status the installer can
    // read.
    #[cfg(windows)]
    if let Some(code) = clispeak_app_lib::handle_path_flag() {
        std::process::exit(code);
    }

    clispeak_app_lib::run();
}
