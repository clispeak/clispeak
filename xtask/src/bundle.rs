//! Building the desktop app with everything it needs inside it.
//!
//! A normal Mac install is a drag into /Applications, after which nothing may
//! be missing: no Homebrew, no downloads on first run, no separate daemon.
//! So Piper, a voice, and the command-line tool are all staged into the
//! bundle before Tauri packages it.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::piper;

/// Build the app, carrying Piper, a voice and the CLI inside it.
pub fn bundle(root: &Path) -> Result<()> {
    let app = root.join("app");
    let tauri = app.join("src-tauri");

    // The command-line tool travels inside the app. It is what an agent
    // actually calls, so shipping the app without it would leave the headline
    // use case needing a second, separate install.
    stage_cli(root, &tauri)?;

    // Piper and one voice, bundled rather than fetched on first run — the
    // same reasoning as the Flatpak manifest, and doubly so on a Mac, which
    // has no system speech this project can reach.
    //
    // `speech`, not `voicecast`: Tauri stages resources beside the built
    // executable, and `target/release/voicecast` is already the command-line
    // tool. A directory of the same name collides with that file and the
    // build dies with a bare "Not a directory".
    piper::fetch(&tauri.join("speech"))?;

    // `tauri.bundle.conf.json` rather than the main config, because Tauri's
    // build script checks that every declared resource and sidecar exists —
    // on *every* `cargo check`, not only when bundling. Declaring them in
    // `tauri.conf.json` would make `cargo build --workspace` fail on any
    // machine that had not staged them first, CI included, which is exactly
    // the five-target rule this repo is built around. The name is not one of
    // Tauri's platform suffixes, so it is merged only when asked for here.
    println!("building the app bundle");
    let status = Command::new("npx")
        .args([
            "tauri",
            "build",
            "--config",
            "src-tauri/tauri.bundle.conf.json",
        ])
        .current_dir(&app)
        .status()
        .context("running `npx tauri build` — is `npm install` done in app/?")?;
    if !status.success() {
        bail!("the app bundle failed to build");
    }
    Ok(())
}

/// Build the CLI in release and put it where Tauri expects a sidecar.
///
/// Tauri names external binaries by target triple and strips the suffix when
/// it copies them into the bundle, which is why the staged file is renamed
/// rather than symlinked.
fn stage_cli(root: &Path, tauri: &Path) -> Result<()> {
    println!("building the command-line tool");
    let status = Command::new("cargo")
        .args(["build", "--release", "-p", "voicecast-cli"])
        .current_dir(root)
        .status()
        .context("running cargo")?;
    if !status.success() {
        bail!("the command-line tool failed to build");
    }

    let built = root.join("target/release/voicecast");
    if !built.exists() {
        bail!("{} was not produced", built.display());
    }

    let binaries = tauri.join("binaries");
    std::fs::create_dir_all(&binaries).context("creating the sidecar directory")?;
    let staged = binaries.join(format!("voicecast-{}", host_triple()?));
    std::fs::copy(&built, &staged).with_context(|| format!("staging {}", staged.display()))?;
    println!("staged  {}", staged.display());
    Ok(())
}

/// The target triple this machine builds for.
fn host_triple() -> Result<String> {
    let out = Command::new("rustc")
        .arg("-vV")
        .output()
        .context("running rustc")?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|l| l.strip_prefix("host: "))
        .map(str::to_string)
        .context("rustc did not report a host triple")
}

/// The repository root, from this crate's location.
pub fn workspace_root() -> Result<PathBuf> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .context("xtask has no parent directory")
}
