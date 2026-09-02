//! Generates Tauri's context at build time.

fn main() {
    // Cargo has no idea the frontend is an input, so editing HTML or JS left
    // the binary embedding a stale copy — the app kept running old code while
    // the build reported success. Declaring them makes a change rebuild.
    for path in ["../src/index.html", "../src/main.js", "../src/styles.css"] {
        println!("cargo:rerun-if-changed={path}");
    }

    tauri_build::build();
}
