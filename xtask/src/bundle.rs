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

/// What npm's runner is actually called.
///
/// Windows ships it as a `.cmd` shim, and `Command::new` there goes straight
/// to `CreateProcess`, which does not consult `PATHEXT` for a bare name. So
/// spawning "npx" fails with "program not found" on a machine where npx is
/// plainly installed and works from a prompt — the same trap the engine hit
/// looking for audio players.
#[cfg(windows)]
const NPX: &str = "npx.cmd";
#[cfg(not(windows))]
const NPX: &str = "npx";

/// The two architectures a shipped Mac has to run on.
///
/// **Intel is not legacy here.** Apple sold Intel Macs until 2023 and supports
/// them still; "Apple silicon only" is a choice to exclude a machine somebody
/// is using today, not a platform that has gone away.
///
/// Until this existed, `x86_64-apple-darwin` appeared nowhere in this
/// repository — not in the release, and not in the compile matrix either. The
/// five-target rule says `macos` and had always meant one of the two Macs
/// Apple sells, which is the kind of gap this project is supposed to catch:
/// the name covered the case and the build never did.
const MAC_ARCHES: [&str; 2] = ["aarch64-apple-darwin", "x86_64-apple-darwin"];

/// What Tauri calls the fat build, and where it puts its output.
const MAC_UNIVERSAL: &str = "universal-apple-darwin";

/// Build the app, carrying Piper, a voice and the CLI inside it.
pub fn bundle(root: &Path) -> Result<()> {
    let app = root.join("app");
    let tauri = app.join("src-tauri");

    // Said before the build rather than after, because the alternative is a
    // bundle that silently came out ad-hoc.
    //
    // `APPLE_SIGNING_IDENTITY` unset falls back to an ad-hoc signature, which
    // runs fine and is invisible in the output. That cost an hour: a shell
    // that predated the `export` produced an unsigned build in the middle of
    // testing whether a Developer ID certificate stops the keychain prompt,
    // and the answer would have been "no" from a test that never signed
    // anything. The variable is read here *and* by Tauri, so one line covers
    // both and says which of the two things happened (#29).
    match std::env::var("APPLE_SIGNING_IDENTITY") {
        Ok(id) if !id.trim().is_empty() => println!("signing  as {id}"),
        _ => {
            println!("signing  AD-HOC — APPLE_SIGNING_IDENTITY is unset.");
            println!("         The bundle runs, and carries no certificate: the");
            println!("         keychain grant holding the device identity is asked");
            println!("         for again after every rebuild (#29).");
        }
    }

    // The command-line tool travels inside the app. It is what an agent
    // actually calls, so shipping the app without it would leave the headline
    // use case needing a second, separate install.
    stage_cli(root, &tauri)?;

    // Piper and one voice, bundled rather than fetched on first run — the
    // same reasoning as the Flatpak manifest.
    //
    // **Not on macOS.** It speaks through `AVSpeechSynthesizer` now, so the
    // payload would be 200MB of engine nothing calls — and it is not inert
    // freight: espeak-ng is GPL-3.0, and shipping it is a redistribution with
    // obligations for something the Mac does not use. That is the licensing
    // half of #132, and switching the engine alone does not deliver it: the
    // bundler stages the payload whether or not anything reads it, which is
    // exactly what a first build with the new engine showed — 208MB with
    // `libespeak-ng.1.dylib` still inside.
    //
    // **Not on Windows either, since 4 September 2026.** Windows speaks
    // through SAPI now, for reasons that were never really about size:
    // Piper links a Visual C++ runtime Windows does not ship, so a clean
    // machine installed it correctly and exited `0xC0000135` with no message
    // (#20). Same conclusion as macOS by a different road.
    //
    // So the condition is not a list of exceptions but a property: **stage
    // the payload where Piper is the engine.** That is Linux and the
    // remaining unixes, and writing it that way means the next platform to
    // move does not have to be remembered here.
    //
    // `speech`, not `clispeak`: Tauri stages resources beside the built
    // executable, and `target/release/clispeak` is already the command-line
    // tool. A directory of the same name collides with that file and the
    // build dies with a bare "Not a directory".
    let piper_is_the_engine = cfg!(all(
        unix,
        not(any(
            target_os = "macos",
            target_os = "ios",
            target_os = "android"
        ))
    ));
    if piper_is_the_engine {
        piper::fetch(&tauri.join("speech"))?;
    }

    // `tauri.bundle.conf.json` rather than the main config, because Tauri's
    // build script checks that every declared resource and sidecar exists —
    // on *every* `cargo check`, not only when bundling. Declaring them in
    // `tauri.conf.json` would make `cargo build --workspace` fail on any
    // machine that had not staged them first, CI included, which is exactly
    // the five-target rule this repo is built around. The name is not one of
    // Tauri's platform suffixes, so it is merged only when asked for here.
    println!("building the app bundle");
    let mut build = Command::new(NPX);
    build.args(["tauri", "build"]);
    // **One download, not a choice to get wrong.** A universal binary runs on
    // both Macs, so the site keeps one button and nobody has to know which
    // processor they have — which is the question a person is least able to
    // answer and most likely to answer wrongly.
    //
    // It costs build time on the most expensive runner GitHub sells, and
    // roughly doubles the executable inside the bundle. Two downloads would
    // cost neither, and would put the burden on the one person in the
    // transaction who cannot check.
    if cfg!(target_os = "macos") {
        build.args(["--target", MAC_UNIVERSAL]);
    }
    let status = build
        .args([
            "--config",
            // A platform that stages no payload declares no speech
            // resource. A declared resource that is absent fails Tauri's own
            // build-script check, so removing the payload and leaving the
            // declaration would break the build rather than shrink it.
            //
            // The file was `tauri.bundle.macos.conf.json` for the few hours
            // macOS was the only platform in this state. Windows joining it
            // made the name wrong rather than merely narrow, which is the
            // kind of stale name this project has paid for before.
            if piper_is_the_engine {
                "src-tauri/tauri.bundle.conf.json"
            } else {
                "src-tauri/tauri.bundle.nopayload.conf.json"
            },
        ])
        .current_dir(&app)
        .status()
        .with_context(|| format!("running `{NPX} tauri build` — is `npm install` done in app/?"))?;
    if !status.success() {
        bail!("the app bundle failed to build");
    }

    // **Checked, because "built universal" and "is universal" are different
    // claims.** A missing rustup target, a Tauri flag that stopped being
    // honoured, or a sidecar that was thin all along each produce a bundle
    // that builds, signs, notarises and installs — and then refuses to open
    // on half the Macs it was built for. The failure arrives as a person
    // saying the app does not work, days later.
    if cfg!(target_os = "macos") {
        let app_binary = root.join(format!(
            "target/{MAC_UNIVERSAL}/release/bundle/macos/clispeak.app/Contents/MacOS/clispeak-app"
        ));
        require_universal(&app_binary)?;
    }
    Ok(())
}

/// Fail unless this Mach-O carries both architectures.
///
/// Reads the file rather than trusting the build that wrote it, for the same
/// reason the Windows import table is read after a release: what an artefact
/// contains is checkable, and not checking it is how "build-verified" becomes
/// "nobody looked" (#201).
fn require_universal(binary: &Path) -> Result<()> {
    if !binary.exists() {
        bail!("{} was not produced", binary.display());
    }
    let out = Command::new("lipo")
        .arg("-archs")
        .arg(binary)
        .output()
        .with_context(|| format!("running lipo -archs on {}", binary.display()))?;
    let archs = String::from_utf8_lossy(&out.stdout);
    let archs: Vec<&str> = archs.split_whitespace().collect();
    // `lipo` spells them as architectures, not triples.
    for wanted in ["arm64", "x86_64"] {
        if !archs.contains(&wanted) {
            bail!(
                "{} is missing {wanted} — it carries [{}].\n\
                 A Mac build that runs on one of the two Macs Apple sells is \n\
                 not a Mac build. Check that both rustup targets are installed:\n\
                   rustup target add {}",
                binary.display(),
                archs.join(", "),
                MAC_ARCHES.join(" ")
            );
        }
    }
    println!("universal  {} [{}]", binary.display(), archs.join(", "));
    Ok(())
}

/// Build the CLI in release and put it where Tauri expects a sidecar.
///
/// Tauri names external binaries by target triple and strips the suffix when
/// it copies them into the bundle, which is why the staged file is renamed
/// rather than symlinked.
fn stage_cli(root: &Path, tauri: &Path) -> Result<()> {
    // The Mac ships one binary that runs on both, so the tool inside it has
    // to be fat as well. A universal app carrying a thin sidecar installs
    // cleanly and then cannot run its own command-line tool on one of the two
    // architectures — and the app works, so nothing points at the sidecar.
    if cfg!(target_os = "macos") {
        return stage_universal_cli(root, tauri);
    }
    println!("building the command-line tool");

    // `.exe` on Windows and nothing anywhere else. From the standard library
    // rather than a conditional of our own, since it is exactly this question.
    let exe = std::env::consts::EXE_SUFFIX;
    let triple = host_triple()?;

    let mut cargo = Command::new("cargo");
    cargo
        .args(["build", "--release", "-p", "clispeak-cli"])
        .current_dir(root);

    // **On Windows the C runtime goes inside the binary.** Without this the
    // tool imports `VCRUNTIME140.dll`, which a clean Windows install does not
    // have — and the failure is total silence: the file is found, the process
    // starts, and it dies before `main` with exit `0xC0000135` and no output
    // at all. Measured on a fresh VM on 6 September 2026, and confirmed by
    // reading the import table of the shipped `clispeak.exe` beside
    // `clispeak-app.exe`, which needs no such library and therefore ran fine.
    //
    // Issue #30 wrote this failure down, to the exact exit code, about Piper.
    // Piper is Linux-only now, so that particular risk went away — and the
    // dependency arrived through our own binary instead, on a path nobody was
    // watching. The lesson was about the *symptom*, not about Piper.
    //
    // **`--target` is not decoration.** `RUSTFLAGS` reaches build scripts and
    // proc-macro crates as well, and a proc macro is a dynamic library that
    // cannot be built against a static CRT. Naming the target explicitly is
    // what confines the flag to the artefact being shipped, which is the
    // documented way round it.
    let built = if cfg!(windows) {
        let mut flags = std::env::var("RUSTFLAGS").unwrap_or_default();
        if !flags.is_empty() {
            flags.push(' ');
        }
        flags.push_str("-C target-feature=+crt-static");
        cargo.args(["--target", &triple]).env("RUSTFLAGS", flags);
        root.join(format!("target/{triple}/release/clispeak{exe}"))
    } else {
        root.join(format!("target/release/clispeak{exe}"))
    };

    let status = cargo.status().context("running cargo")?;
    if !status.success() {
        bail!("the command-line tool failed to build");
    }

    if !built.exists() {
        bail!("{} was not produced", built.display());
    }

    let binaries = tauri.join("binaries");
    std::fs::create_dir_all(&binaries).context("creating the sidecar directory")?;
    // Tauri looks for a sidecar named for the target triple, and keeps the
    // platform's executable suffix. Without it Windows stages a file nothing
    // will run, beside a `built` path that never existed to copy from.
    let staged = binaries.join(format!("clispeak-{triple}{exe}"));
    std::fs::copy(&built, &staged).with_context(|| format!("staging {}", staged.display()))?;
    println!("staged  {}", staged.display());
    Ok(())
}

/// Build the command-line tool for both Macs and lipo them together.
///
/// **Three names are staged, deliberately.** Tauri finds a sidecar by
/// appending a target triple, and which triple it appends for a universal
/// build is a detail of a version we pin with a caret. Staging the universal
/// name *and* both per-architecture names means the build works whichever it
/// asks for, and the alternative is a release that fails at the packaging
/// step for a reason nobody can see from here.
///
/// It costs two copies of a 3MB binary in a build directory.
fn stage_universal_cli(root: &Path, tauri: &Path) -> Result<()> {
    let binaries = tauri.join("binaries");
    std::fs::create_dir_all(&binaries).context("creating the sidecar directory")?;

    let mut thin = Vec::new();
    for arch in MAC_ARCHES {
        println!("building the command-line tool for {arch}");
        let status = Command::new("cargo")
            .args(["build", "--release", "-p", "clispeak-cli", "--target", arch])
            .current_dir(root)
            .status()
            .context("running cargo")?;
        if !status.success() {
            bail!(
                "the command-line tool failed to build for {arch}.\n\
                 If the target is missing:  rustup target add {arch}"
            );
        }
        let built = root.join(format!("target/{arch}/release/clispeak"));
        if !built.exists() {
            bail!("{} was not produced", built.display());
        }
        // Staged under its own triple as well as the fat one, so whichever
        // name Tauri reaches for is there.
        let staged = binaries.join(format!("clispeak-{arch}"));
        std::fs::copy(&built, &staged).with_context(|| format!("staging {}", staged.display()))?;
        thin.push(built);
    }

    let fat = binaries.join(format!("clispeak-{MAC_UNIVERSAL}"));
    let status = Command::new("lipo")
        .arg("-create")
        .args(&thin)
        .arg("-output")
        .arg(&fat)
        .status()
        .context("running lipo -create — it ships with the Xcode command line tools")?;
    if !status.success() {
        bail!("lipo could not combine the two builds of the command-line tool");
    }
    require_universal(&fat)?;
    println!("staged  {}", fat.display());
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
