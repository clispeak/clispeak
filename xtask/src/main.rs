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

    println!("portability ok: {} crates clean", PORTABLE.len());
    Ok(())
}

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
