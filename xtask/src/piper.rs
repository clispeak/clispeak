//! Fetching Piper, its libraries, and a voice.
//!
//! Piper is downloaded rather than built: it is a large C++ project with an
//! ONNX runtime behind it, and the upstream releases are already the binaries
//! the Flatpak manifest pins. Everything here is checksummed against those
//! same pins, so a tampered or truncated download fails loudly instead of
//! being packaged.
//!
//! The macOS release is **incomplete**: `piper_macos_*.tar.gz` ships the
//! executable with no `LC_RPATH` and without the three dylibs it links
//! against — only a `.dSYM` for one of them. The libraries live in the
//! separate `piper-phonemize` release instead. So on macOS this task merges
//! the two archives, adds an rpath of `@loader_path` so the binary finds the
//! dylibs sitting beside it, and re-signs: an arm64 Mach-O will not load at
//! all once its signature is invalidated by the rpath edit.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

/// The upstream release every pin below refers to.
const PIPER_TAG: &str = "2023.11.14-2";
/// The phonemize release carrying the macOS dylibs.
const PHONEMIZE_TAG: &str = "2023.11.14-4";
/// The voice the app ships with, matching the Flatpak manifest.
///
/// **Public domain, and that is why it is this one.** The previous default,
/// `en_US-lessac-medium`, is trained on the Blizzard Challenge 2013 Lessac
/// corpus, whose licence grants use "exclusively for Research Purposes only"
/// and bars distribution outright — so it could not be shipped in anything,
/// and downloading it for a user is not obviously better than shipping it.
/// LJ Speech has no restrictions at all: "There are no restrictions on its
/// use... you may use it without attribution" (decision 79).
const VOICE: &str = "en_US-ljspeech-medium";

/// One downloaded file and the hash it must have.
struct Pinned {
    url: String,
    sha256: &'static str,
    name: &'static str,
}

/// The Piper build for the host, or an explanation of why there is no pin.
///
/// Unpinned platforms are refused rather than downloaded: a checksum that
/// nobody has verified is worth nothing, and silently packaging an
/// unverified binary is the failure this whole task exists to prevent.
fn piper_for_host() -> Result<(Pinned, Option<Pinned>)> {
    let (piper, phonemize) = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => (
            (
                "piper_macos_aarch64.tar.gz",
                "6b1eb03b3735946cb35216e063e7eebcc33a6bbf5dd96ec0217959bf1cdcb0cc",
            ),
            // Only macOS needs the second archive; the Linux release is
            // self-contained.
            Some((
                "piper-phonemize_macos_aarch64.tar.gz",
                "78a9c28b3c94baf6e9526b2e386ce547909abaec4f31aadd7e16b01fbfe5f322",
            )),
        ),
        ("linux", "x86_64") => (
            (
                "piper_linux_x86_64.tar.gz",
                "a50cb45f355b7af1f6d758c1b360717877ba0a398cc8cbe6d2a7a3a26e225992",
            ),
            None,
        ),
        // Self-contained, unlike the macOS release: the archive carries
        // onnxruntime, piper_phonemize, espeak-ng and its data alongside the
        // executable, so there is nothing to merge and no signature to
        // repair. Checked by hand against the release on 2026-09-02.
        ("windows", "x86_64") => (
            (
                "piper_windows_amd64.zip",
                "f3c58906402b24f3a96d92145f58acba6d86c9b5db896d207f78dc80811efcea",
            ),
            None,
        ),
        (os, arch) => bail!(
            "no verified Piper pin for {os}/{arch}. Add one to xtask/src/piper.rs \
             after checking the download by hand — an unverified binary is not \
             worth packaging."
        ),
    };

    let piper = Pinned {
        url: format!(
            "https://github.com/rhasspy/piper/releases/download/{PIPER_TAG}/{}",
            piper.0
        ),
        sha256: piper.1,
        name: piper.0,
    };
    let phonemize = phonemize.map(|(name, sha256)| Pinned {
        url: format!(
            "https://github.com/rhasspy/piper-phonemize/releases/download/{PHONEMIZE_TAG}/{name}"
        ),
        sha256,
        name,
    });
    Ok((piper, phonemize))
}

/// The voice model and its sidecar config.
fn voice_files() -> [Pinned; 2] {
    let base = format!(
        "https://huggingface.co/rhasspy/piper-voices/resolve/v1.0.0/en/en_US/ljspeech/medium/{VOICE}"
    );
    [
        Pinned {
            url: format!("{base}.onnx"),
            sha256: "6f52a751e2349abe7a76735eb09dc1875298c77ea2342ffd2fef79ff81b87f22",
            name: "en_US-ljspeech-medium.onnx",
        },
        Pinned {
            url: format!("{base}.onnx.json"),
            sha256: "141d612cc0a95ed7efc1ca936b845c2364967f2e9217c5dbfcf69fc4d6c65860",
            name: "en_US-ljspeech-medium.onnx.json",
        },
    ]
}

/// Put Piper and a voice under `root`, laid out as the engine expects.
///
/// `root/piper/piper` is the binary and `root/voices/*.onnx` the models —
/// the same shape whether this is the user's data directory or a staging
/// directory about to be copied into an application bundle.
pub fn fetch(root: &Path) -> Result<()> {
    let cache = download_cache()?;
    std::fs::create_dir_all(root.join("voices")).context("creating the voices directory")?;

    let (piper, phonemize) = piper_for_host()?;

    let archive = download(&cache, &piper)?;
    // Extracting over an existing copy would leave a half-replaced mixture of
    // two versions behind, so the old one goes first.
    let piper_dir = root.join("piper");
    if piper_dir.exists() {
        std::fs::remove_dir_all(&piper_dir).context("removing the previous Piper")?;
    }
    extract(&archive, root)?;

    if let Some(phonemize) = phonemize {
        let archive = download(&cache, &phonemize)?;
        let staged = cache.join("phonemize");
        if staged.exists() {
            std::fs::remove_dir_all(&staged).ok();
        }
        std::fs::create_dir_all(&staged)?;
        extract(&archive, &staged)?;
        merge_libraries(&staged.join("piper-phonemize/lib"), &piper_dir)?;
        prune(&piper_dir)?;
        repair_macho(&piper_dir.join("piper"))?;
    }

    for file in voice_files() {
        let downloaded = download(&cache, &file)?;
        std::fs::copy(&downloaded, root.join("voices").join(file.name))
            .with_context(|| format!("installing {}", file.name))?;
    }

    println!("piper ready in {}", root.display());
    Ok(())
}

/// Where downloaded archives are kept between runs.
///
/// Under the user's own directory rather than the shared temp directory. Every
/// file here is pinned by SHA-256 and checked, but the check and the use are
/// two separate reads of the same path: `/tmp/voicecast-piper-downloads` can
/// be created in advance by another local user, who can then swap the archive
/// between the two. `curl --output` would also write straight through a
/// planted symlink. What comes out is installed as `piper`, a binary this
/// node then executes and `xtask bundle` ships inside a `.app` (#67).
///
/// Owner-only, because the point is that nobody else can put anything here.
fn download_cache() -> Result<PathBuf> {
    let cache = user_root()?.join("downloads");
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(&cache)
        .context("creating the download cache")?;
    Ok(cache)
}

/// Fetch a pinned file into `cache`, reusing it if the hash already matches.
fn download(cache: &Path, file: &Pinned) -> Result<PathBuf> {
    let path = cache.join(file.name);
    if path.exists() && sha256(&path)? == file.sha256 {
        println!("cached  {}", file.name);
        return Ok(path);
    }

    println!("fetch   {}", file.name);
    let status = Command::new("curl")
        .args([
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            "--output",
        ])
        .arg(&path)
        .arg(&file.url)
        .status()
        .context("running curl")?;
    if !status.success() {
        bail!("could not download {}", file.url);
    }

    let got = sha256(&path)?;
    if got != file.sha256 {
        // Leaving the file would make the next run trust a bad copy.
        std::fs::remove_file(&path).ok();
        bail!(
            "{} has checksum {got}, expected {}. The release may have been \
             re-published, or the download was tampered with.",
            file.name,
            file.sha256
        );
    }
    Ok(path)
}

/// A file's SHA-256, as lowercase hex.
///
/// Shelled out because the three platforms spell it differently and none of
/// them needs a crate to do it.
fn sha256(path: &Path) -> Result<String> {
    let mut command = Command::new(if cfg!(windows) {
        "certutil"
    } else if cfg!(target_os = "macos") {
        "shasum"
    } else {
        "sha256sum"
    });
    if cfg!(target_os = "macos") {
        command.args(["-a", "256"]);
    }
    if cfg!(windows) {
        command.arg("-hashfile");
    }
    command.arg(path);
    // certutil names the algorithm after the file rather than before it.
    if cfg!(windows) {
        command.arg("SHA256");
    }

    let out = command.output().context("hashing the download")?;
    if !out.status.success() {
        bail!("could not hash {}", path.display());
    }

    // All three print the digest as a bare 64-character hex token, but not in
    // the same place: sha256sum and shasum put it first, certutil puts it on
    // its own line beneath a heading. Finding it by shape rather than by
    // position is what lets one parser serve all three — and a parser that
    // silently returned the wrong token would turn the checksum gate into
    // theatre, which is worse than not having it.
    let text = String::from_utf8_lossy(&out.stdout);
    text.split_whitespace()
        .find(|token| token.len() == 64 && token.chars().all(|c| c.is_ascii_hexdigit()))
        .map(str::to_ascii_lowercase)
        .with_context(|| format!("no digest found in the output for {}", path.display()))
}

/// Unpack an archive into `into`.
///
/// One command for both shapes: Windows 10 and later ship bsdtar as `tar`,
/// which reads zip archives as happily as tarballs. The flags differ only in
/// whether the archive is gzipped.
fn extract(archive: &Path, into: &Path) -> Result<()> {
    let gzipped = archive.extension().is_some_and(|e| e == "gz");
    let status = Command::new("tar")
        .arg(if gzipped { "xzf" } else { "xf" })
        .arg(archive)
        .arg("-C")
        .arg(into)
        .status()
        .context("running tar")?;
    if !status.success() {
        bail!("could not extract {}", archive.display());
    }
    Ok(())
}

/// The three libraries Piper actually links against, by `otool -L`.
///
/// Named rather than copied wholesale: the release also carries unversioned
/// aliases of each, which are ordinarily symlinks but arrive here as full
/// copies, and one of them is a 24MB ONNX runtime. Taking only these keeps
/// the bundle around 40MB instead of 82MB.
const LIBRARIES: &[&str] = &[
    "libespeak-ng.1.dylib",
    "libpiper_phonemize.1.dylib",
    "libonnxruntime.1.14.1.dylib",
];

/// Everything else Piper needs beside its libraries.
///
/// `espeak-ng-data` is the phonemiser's dictionaries, and `libtashkeel_model.ort`
/// restores Arabic diacritics. Anything not named here or in [`LIBRARIES`] is
/// removed: the standalone `espeak-ng` and `piper_phonemize` executables are
/// unused, `pkgconfig` is for building against Piper rather than running it,
/// and the `.dSYM` is debug symbols.
const KEEP: &[&str] = &["piper", "espeak-ng-data", "libtashkeel_model.ort"];

/// Copy the dylibs Piper links against in beside the binary.
fn merge_libraries(from: &Path, into: &Path) -> Result<()> {
    for name in LIBRARIES {
        let source = from.join(name);
        if !source.exists() {
            bail!("{} is missing from {}", name, from.display());
        }
        std::fs::copy(&source, into.join(name))
            .with_context(|| format!("copying {}", source.display()))?;
    }
    println!(
        "merged  {} libraries into {}",
        LIBRARIES.len(),
        into.display()
    );
    Ok(())
}

/// Drop everything from the Piper directory that running it does not need.
///
/// Also the fix for a packaging failure rather than only a size saving: the
/// release ships two empty directories, and Tauri's resource walker stops on
/// them with a bare "Not a directory".
fn prune(dir: &Path) -> Result<()> {
    let mut removed = 0;
    for entry in std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .flatten()
    {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if LIBRARIES.contains(&name) || KEEP.contains(&name) {
            continue;
        }
        if path.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        }
        .with_context(|| format!("removing {}", path.display()))?;
        removed += 1;
    }
    println!("pruned  {removed} unused entries from {}", dir.display());
    Ok(())
}

/// The identity everything in the bundle is signed with.
///
/// `APPLE_SIGNING_IDENTITY` when set, which is also the variable Tauri reads
/// for the app itself, so one setting covers the whole bundle. Otherwise an
/// ad-hoc signature, which is enough to *run* locally but has a real cost:
/// an ad-hoc identity is derived from the binary's own hash, so every rebuild
/// looks like a different program to macOS. The keychain grant that stores
/// the device identity is made to a specific one, so "Always Allow" is
/// forgotten on the next build and the password is asked for again. A stable
/// certificate — even a self-signed one — is what stops that.
fn signing_identity() -> String {
    std::env::var("APPLE_SIGNING_IDENTITY").unwrap_or_else(|_| "-".to_string())
}

/// Sign one Mach-O file in place, the way notarisation requires.
///
/// **`--options runtime` and a secure timestamp, or Apple rejects the whole
/// bundle.** These four Mach-O files live in `Contents/Resources/speech/`,
/// and `tauri-bundler` signs nested code only in `MacOS`, `Frameworks`,
/// `Plugins`, `Helpers`, `XPCServices` and `Libraries` — `Resources` is not
/// on that list. So this function is the *only* thing that signs them, and it
/// used to pass `--timestamp=none` with no hardened runtime at all. Measured
/// on a Developer ID build before the fix: the two executables in `MacOS`
/// carried `flags=0x10000(runtime)` and a timestamp, and all four of these
/// carried neither (#119).
///
/// Nothing would have said so. The bundle signs, `codesign -v` passes, it
/// runs — and the rejection arrives from Apple during a release, which is the
/// most expensive possible moment to learn it.
///
/// A timestamp needs Apple's timestamp server, so this is the one signing
/// step that requires the network. Ad-hoc signatures cannot be timestamped at
/// all, so that path keeps `--timestamp=none`: an ad-hoc bundle is not going
/// to be notarised, and failing the local build over it would be a gate
/// against something nobody was attempting.
///
/// **The hardened runtime is conditional too, and that one is not a nicety.**
/// It enables library validation, which requires a loaded library to carry
/// the same team identifier as the process loading it. `piper` is spawned as
/// its own process and loads three dylibs from beside itself through the
/// `LC_RPATH` written above. Under a Developer ID they share a team and it
/// works — measured. Under an ad-hoc signature *neither has a team at all*,
/// and macOS does not treat that as a match:
///
/// ```text
/// Library not loaded: @rpath/libespeak-ng.1.dylib
/// Reason: code signature not valid for use in process:
///         mapping process and mapped file (non-platform) have different Team IDs
/// ```
///
/// So applying it unconditionally would have made the app mute for every
/// build without a certificate — every other developer's local build, and the
/// artefact CI produces today, since `release.yml` builds unsigned while no
/// secrets are set. A strictly wider blast radius than the notarisation
/// rejection it fixes.
///
/// The cost of the condition is real and worth naming: a local build no
/// longer exercises the loader restrictions the shipped one has, so a
/// hardened-runtime problem can now only be found on a signed build. That is
/// the trade, and it is the better half of it — a signed build is testable
/// here, and a mute app on every unsigned build is not a thing to trade for
/// coverage.
fn sign(path: &Path) -> Result<()> {
    let identity = signing_identity();
    let adhoc = identity == "-";
    // Both are conditional on a real certificate, for different reasons, and
    // the second one is the reason this function has a long comment.
    let mut args = vec!["--force"];
    if adhoc {
        // An ad-hoc signature cannot be timestamped.
        args.push("--timestamp=none");
    } else {
        args.push("--timestamp");
        args.push("--options");
        args.push("runtime");
    }
    args.push("--sign");
    args.push(&identity);
    let status = Command::new("codesign")
        .args(&args)
        .arg(path)
        .status()
        .context("running codesign")?;
    if !status.success() {
        bail!("could not sign {}", path.display());
    }
    Ok(())
}

/// Teach the binary to find the dylibs beside it, and re-sign it.
///
/// Upstream ships no `LC_RPATH` at all, so the loader only ever looks in
/// `/usr/local/lib` and `/usr/lib` and the binary dies before `main`. Editing
/// the load commands invalidates the code signature, and macOS on arm64
/// refuses to run a Mach-O with a broken one — hence the re-sign.
///
/// The libraries are signed too. Nothing needs that to run today, but a
/// notarised or App Store build requires every Mach-O inside the bundle to
/// carry the same identity, and signing them here keeps that one setting
/// rather than a separate pass to remember later.
fn repair_macho(binary: &Path) -> Result<()> {
    // Re-running the task would otherwise stack up duplicate rpaths.
    let existing = Command::new("otool")
        .args(["-l"])
        .arg(binary)
        .output()
        .context("running otool")?;
    if !String::from_utf8_lossy(&existing.stdout).contains("path @loader_path ") {
        let status = Command::new("install_name_tool")
            .args(["-add_rpath", "@loader_path"])
            .arg(binary)
            .status()
            .context("running install_name_tool")?;
        if !status.success() {
            bail!("could not add an rpath to {}", binary.display());
        }
    }

    // Libraries first: signing the binary seals what it loads, so a library
    // re-signed afterwards would invalidate the binary's own signature.
    let Some(dir) = binary.parent() else {
        bail!("{} has no parent directory", binary.display());
    };
    for library in LIBRARIES {
        sign(&dir.join(library))?;
    }
    sign(binary)?;

    let identity = signing_identity();
    let described = if identity == "-" {
        "ad-hoc".to_string()
    } else {
        identity
    };
    println!(
        "signed  piper and {} libraries ({described})",
        LIBRARIES.len()
    );
    Ok(())
}

/// Where a developer's own Piper lives, matching the engine's first root.
///
/// Spelled out per platform rather than pulled from `directories`, which is
/// what the engine uses: this crate depends on one thing and this is the only
/// place it would be needed. The two must agree, so a change here belongs
/// with a change to `install_roots`.
pub fn user_root() -> Result<PathBuf> {
    // Windows has no HOME, and the equivalent is not under one either.
    if cfg!(windows) {
        let local = std::env::var_os("LOCALAPPDATA").context("no LOCALAPPDATA")?;
        return Ok(PathBuf::from(local).join("voicecast"));
    }
    let home = std::env::var_os("HOME").context("no HOME")?;
    let home = PathBuf::from(home);
    Ok(if cfg!(target_os = "macos") {
        home.join("Library/Application Support/voicecast")
    } else {
        home.join(".local/share/voicecast")
    })
}
