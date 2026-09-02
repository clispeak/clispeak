//! Desktop entry point. Mobile enters through `lib.rs` instead.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    voicecast_app_lib::run();
}
