//! Build automation, run as `cargo xtask <task>`.

mod bundle;
mod piper;

use std::path::{Path, PathBuf};

/// Crates that must stay portable across all five targets.
const PORTABLE: &[&str] = &["voicecast-proto", "voicecast-text", "voicecast-core"];

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("portability") => portability(),
        // Puts Piper and a voice where the engine looks for them, so a
        // freshly cloned checkout can speak without hunting for downloads.
        Some("piper") => {
            let root = match args.next() {
                Some(dir) => PathBuf::from(dir),
                None => piper::user_root()?,
            };
            piper::fetch(&root)
        }
        // Builds the app with Piper, a voice and the CLI inside it, which
        // is what makes a Mac install a single drag with nothing to follow.
        Some("bundle") => bundle::bundle(&bundle::workspace_root()?),
        _ => {
            eprintln!("usage: cargo xtask <portability | piper [dir] | bundle>");
            Ok(())
        }
    }
}

/// Fail if a portable crate has grown a `#[cfg(target_os)]`.
///
/// Mechanical enforcement of the rule in `docs/build-plan.md`. The failure it
/// prevents is months of one platform's assumptions quietly settling into
/// shared code, discovered the week someone first builds for another.
///
/// Comment lines are skipped — the docs legitimately *mention* the attribute,
/// and matching prose would make the gate cry wolf.
fn portability() -> anyhow::Result<()> {
    let mut findings = Vec::new();

    for krate in PORTABLE {
        let dir = PathBuf::from("crates").join(krate).join("src");
        for file in rust_files(&dir)? {
            let text = std::fs::read_to_string(&file)?;
            for (i, line) in text.lines().enumerate() {
                let trimmed = line.trim_start();
                if trimmed.starts_with("//") {
                    continue;
                }
                if line.contains("cfg(target_os") || line.contains("cfg(target_family") {
                    findings.push(format!("  {}:{}: {}", file.display(), i + 1, trimmed));
                }
            }
        }
    }

    if !findings.is_empty() {
        eprintln!("portable crates must not contain platform conditionals:");
        for f in &findings {
            eprintln!("{f}");
        }
        anyhow::bail!("platform code belongs in voicecast-engine or the Tauri shell");
    }

    frontend_dialogs()?;
    let jni = jni_keep_rules()?;

    // The counts are said out loud because a gate that passed and a gate that
    // never ran print the same thing otherwise — which is how the JNI check
    // came to be doubted the day after it was written, reasonably.
    println!(
        "portability ok: {} crates clean, {jni} JNI classes kept",
        PORTABLE.len()
    );
    Ok(())
}

/// Fail if the frontend calls a blocking dialog the webview may not have.
///
/// The same rule as above, one layer up. `window.confirm`, `alert` and
/// `prompt` are not portable: WebKitGTK and WebView2 show them, WKWebView
/// shows one only if the host implements a `WKUIDelegate` for it, and wry
/// implements none. On macOS `confirm()` therefore displayed nothing and
/// returned false, so five destructive actions — clearing the history,
/// removing a device, dropping, leaving and rotating a space — silently did
/// nothing while appearing to succeed.
///
/// Worth a mechanical gate rather than a note, because the symptom is a
/// button that looks like it worked. Use `ask()` in `main.js`, which behaves
/// the same on all five targets.
///
/// Comments are skipped, so the explanation of the rule cannot trip it.
fn frontend_dialogs() -> anyhow::Result<()> {
    let file = PathBuf::from("app/src/main.js");
    let Ok(text) = std::fs::read_to_string(&file) else {
        // Not a checkout with a frontend in it; nothing to say.
        return Ok(());
    };

    let mut findings = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") || trimmed.starts_with("*") || trimmed.starts_with("/*") {
            continue;
        }
        for call in ["confirm(", "alert(", "prompt("] {
            // `window.` prefixed or bare, but not `ask(` or a longer name
            // that merely ends in one of these.
            let found = line.match_indices(call).any(|(at, _)| {
                line[..at]
                    .chars()
                    .next_back()
                    .is_none_or(|c| !c.is_alphanumeric() && c != '_' && c != '.')
                    || line[..at].ends_with("window.")
            });
            if found {
                findings.push(format!("  {}:{}: {}", file.display(), i + 1, trimmed));
                break;
            }
        }
    }

    if !findings.is_empty() {
        eprintln!("the frontend must not use a webview's blocking dialogs:");
        for f in &findings {
            eprintln!("{f}");
        }
        anyhow::bail!("use ask() in main.js — WKWebView shows none of these");
    }
    Ok(())
}

/// Fail if a class Rust looks up over JNI has no ProGuard keep rule.
///
/// R8 runs on release builds only, and it decides what to delete by looking
/// for callers. `voicecast-engine` reaches its Kotlin by *name* —
/// `find_class("com/voicecast/app/Speech")` — which no static analysis can
/// see, so R8 renamed the class and the release APK died on launch with
/// `NoSuchMethodError` while the debug APK, which does not minify, was fine.
///
/// That is the worst shape a bug can have here: every test anyone had run on
/// a real phone was a debug build, so the fault existed only in the artefact
/// built for other people. See issue #41.
///
/// Mechanical because the failure mode is adding a fourth class and finding
/// out from a crash report. Comments are skipped so this file's own
/// explanation cannot satisfy the rule it enforces.
fn jni_keep_rules() -> anyhow::Result<usize> {
    let sources = PathBuf::from("crates/voicecast-engine/src");
    let rules = PathBuf::from("app/src-tauri/gen/android/app/proguard-rules.pro");
    let Ok(text) = std::fs::read_to_string(&rules) else {
        // Not a checkout with an Android project in it; nothing to say.
        return Ok(0);
    };

    // What the rules already cover, as `com.voicecast.app.Speech`.
    let kept: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("-keep"))
        .filter_map(|l| l.split_whitespace().nth(2).map(str::to_string))
        .collect();

    let mut findings = Vec::new();
    let mut checked = 0usize;
    for file in rust_files(&sources)? {
        let body = std::fs::read_to_string(&file)?;
        for (i, line) in body.lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            for (at, _) in line.match_indices(NEEDLE) {
                checked += 1;
                let rest = &line[at + 1..];
                let Some(end) = rest.find(QUOTE) else {
                    continue;
                };
                let dotted = rest[..end].replace('/', ".");
                if !kept.contains(&dotted) {
                    findings.push(format!(
                        "  {}:{}: {dotted} is looked up by name and has no keep rule",
                        file.display(),
                        i + 1
                    ));
                }
            }
        }
    }

    if !findings.is_empty() {
        eprintln!("classes reached over JNI must survive R8:");
        for f in &findings {
            eprintln!("{f}");
        }
        anyhow::bail!(
            "add `-keep class <name> {{ *; }}` to \
             app/src-tauri/gen/android/app/proguard-rules.pro"
        );
    }
    Ok(checked)
}

/// An opening quote followed by the package every JNI lookup starts with.
///
/// Assembled rather than written literally so this file does not contain the
/// pattern it searches for, which would make the gate find itself.
const QUOTE: char = '"';
const NEEDLE: &str = "\"com/voicecast/";

/// Every `.rs` file under `dir`, recursively.
fn rust_files(dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    if !dir.exists() {
        return Ok(out);
    }
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            out.extend(rust_files(&path)?);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    Ok(out)
}
