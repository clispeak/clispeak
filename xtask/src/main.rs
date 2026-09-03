//! Build automation, run as `cargo xtask <task>`.

mod bundle;
mod piper;

use anyhow::Context as _;
use std::path::{Path, PathBuf};

/// Crates that must stay portable across all five targets.
const PORTABLE: &[&str] = &["voicecast-proto", "voicecast-text", "voicecast-core"];

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("portability") => portability(),
        // Everything, in one command, because the alternative is a chain
        // assembled by hand at the shell — and this week that chain has
        // short-circuited so an edit never ran, used `;` where `&&` was meant
        // so a failing gate did not stop a push, and simply omitted clippy so
        // a lint reached `main` (#91). Each was a different mistake with the
        // same shape: the check was fine and the wiring around it was not.
        Some("check") => check(),
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
            eprintln!("usage: cargo xtask <check | portability | piper [dir] | bundle>");
            Ok(())
        }
    }
}

/// Every gate, in order, stopping at the first that fails.
///
/// The order is cheapest-first, so a formatting slip is reported in a second
/// rather than after a full test run. Each step prints its own name before
/// running, because a gate that passed and a gate that never ran otherwise
/// look identical in a scrollback — the same reasoning as the counts printed
/// by `portability`.
fn check() -> anyhow::Result<()> {
    // Before anything cargo does, because every other gate's verdict is
    // meaningless on a tree in this state and would be *reported* anyway.
    // That is the severity: not that markers survive, but that a green
    // result is returned about a file nobody has finished merging (#103).
    eprintln!("== conflicts");
    conflict_markers()?;
    // Also ahead of cargo: it reads two workflow files and costs
    // milliseconds, and a shell continuation that YAML ate is not something
    // any Rust gate can see (#102).
    eprintln!("== workflows");
    workflow_run_steps()?;

    let steps: &[(&str, &[&str])] = &[
        ("fmt", &["fmt", "--all", "--check"]),
        (
            "clippy",
            &[
                "clippy",
                "--workspace",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ],
        ),
        ("test", &["test", "--workspace"]),
    ];
    for (name, args) in steps {
        eprintln!("== {name}");
        let status = std::process::Command::new(cargo())
            .args(*args)
            .status()
            .with_context(|| format!("running cargo {name}"))?;
        if !status.success() {
            anyhow::bail!("{name} failed");
        }
    }
    // Last because it is this binary's own work and needs the others' output
    // in the scrollback above it when it fails.
    eprintln!("== portability");
    portability()?;
    eprintln!();
    eprintln!("all gates passed");
    Ok(())
}

/// The cargo that invoked us, so a rustup shim is not re-resolved from PATH.
fn cargo() -> std::ffi::OsString {
    std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into())
}

/// Fail if a portable crate has grown a `#[cfg(target_os)]`.
///
/// Mechanical enforcement of the rule in `docs/build-plan.md`. The failure it
/// prevents is months of one platform's assumptions quietly settling into
/// shared code, discovered the week someone first builds for another.
///
/// Comment lines are skipped — the docs legitimately *mention* the attribute,
/// and matching prose would make the gate cry wolf.
/// Every spelling of "this code is for one platform".
///
/// The gate checked two of these and printed "3 crates clean", which was a
/// stronger sentence than it had earned: `cfg(unix)` sat in `voicecast-core`
/// the whole time and was never mentioned. The forms that matter are the ones
/// that change what compiles, and `cfg(unix)` with no `cfg(windows)` arm
/// compiles on four targets and fails on the fifth — which is the exact shape
/// of every row in CLAUDE.md's table of divergence that has bitten us (#88).
const CONDITIONALS: &[&str] = &[
    "target_os",
    "target_family",
    "target_arch",
    "target_env",
    "target_pointer_width",
    "unix",
    "windows",
];

/// Whether a line is a platform conditional.
///
/// Matched as a predicate inside a `cfg`, rather than as the literal text
/// `cfg(unix`, because `#[cfg(not(unix))]` and `#[cfg(all(unix, feature =
/// "x"))]` are the same claim wrapped differently and a prefix match sees
/// neither. `unix` and `windows` need a word boundary or every line
/// mentioning a `windows` field would trip.
fn is_platform_conditional(line: &str) -> bool {
    if !line.contains("cfg(") && !line.contains("cfg_attr(") {
        return false;
    }
    CONDITIONALS.iter().any(|needle| {
        line.match_indices(needle).any(|(at, _)| {
            let before = line[..at].chars().next_back();
            let after = line[at + needle.len()..].chars().next();
            let boundary = |c: Option<char>| !c.is_some_and(|c| c.is_alphanumeric() || c == '_');
            boundary(before) && boundary(after)
        })
    })
}

/// What a line must carry, on the line above, to be allowed one.
///
/// Some divergence is unavoidable: setting a file mode has no portable
/// spelling, so `store.rs` needs `cfg(unix)` whatever the rule says. The
/// choice is between an allowlist in this file, which drifts from the code it
/// names, and a marker where the reader already is. The marker wins, and it
/// has to carry a reason — a bare exemption is the thing that gets pasted
/// without thought.
const EXCEPTION: &str = "portability-exception:";

fn portability() -> anyhow::Result<()> {
    let mut findings = Vec::new();
    let mut allowed = 0usize;

    for krate in PORTABLE {
        let dir = PathBuf::from("crates").join(krate).join("src");
        for file in rust_files(&dir)? {
            let text = std::fs::read_to_string(&file)?;
            let lines: Vec<&str> = text.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                let trimmed = line.trim_start();
                if trimmed.starts_with("//") {
                    continue;
                }
                if !is_platform_conditional(line) {
                    continue;
                }
                // Anywhere in the comment block directly above, so the
                // reason sits where it is read and may run to more than one
                // line — which the first version of this could not handle,
                // and which is exactly how long a real reason turns out to be.
                let declared = lines[..i]
                    .iter()
                    .rev()
                    .take_while(|l| {
                        let t = l.trim_start();
                        t.starts_with("//") || t.starts_with("#[") || t.is_empty()
                    })
                    .any(|l| {
                        l.split_once(EXCEPTION)
                            .is_some_and(|(_, why)| !why.trim().is_empty())
                    });
                if declared {
                    allowed += 1;
                } else {
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
        eprintln!();
        eprintln!("Move it to voicecast-engine or the Tauri shell. If it genuinely");
        eprintln!("cannot be expressed portably, say why on the line above:");
        eprintln!("  // {EXCEPTION} <the reason there is no portable spelling>");
        anyhow::bail!("platform code belongs in voicecast-engine or the Tauri shell");
    }

    frontend_dialogs()?;
    let jni = jni_keep_rules()?;
    let decisions = decision_numbers()?;

    // The counts are said out loud because a gate that passed and a gate that
    // never ran print the same thing otherwise — which is how the JNI check
    // came to be doubted the day after it was written, reasonably.
    // The counts are said out loud because a gate that passed and a gate that
    // never ran print the same thing otherwise — and the declared exceptions
    // are counted for the same reason: "clean" that silently means "clean
    // apart from two" is how this check came to overstate itself.
    println!(
        "portability ok: {} crates clean against {} conditional forms \
         ({allowed} declared exception{}), {jni} JNI classes kept, \
         {decisions} decisions numbered",
        PORTABLE.len(),
        CONDITIONALS.len(),
        if allowed == 1 { "" } else { "s" }
    );
    Ok(())
}

/// Fail if `docs/decisions.md` numbers its decisions with a gap or a repeat.
///
/// The file is append-only and cited by number from CLAUDE.md, issues and
/// commit messages, so a number has to name exactly one decision. Two agents
/// appending on parallel branches each picked the next free number, both
/// were 33, and the rebase kept both: for a day "decision 33" meant two
/// things and "decision 34" pointed one past where it was written. Nothing
/// read the sequence, so nothing noticed. Returns how many were checked.
fn decision_numbers() -> anyhow::Result<usize> {
    let path = Path::new("docs/decisions.md");
    let text = std::fs::read_to_string(path)?;
    let mut expected = 1usize;
    let mut findings = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let Some(rest) = line.strip_prefix("## ") else {
            continue;
        };
        let Some((number, _)) = rest.split_once(". ") else {
            continue;
        };
        let Ok(number) = number.trim().parse::<usize>() else {
            continue;
        };
        if number != expected {
            findings.push(format!(
                "  {}:{}: decision {number}, expected {expected}",
                path.display(),
                i + 1
            ));
            // Resume from what was found, so one slip reports once rather
            // than as every heading after it.
            expected = number;
        }
        expected += 1;
    }
    if !findings.is_empty() {
        eprintln!("decisions must be numbered consecutively, each number once:");
        for f in &findings {
            eprintln!("{f}");
        }
        anyhow::bail!("renumber the later decision and update anything that cites it");
    }
    Ok(expected - 1)
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
/// Files allowed to contain what looks like a conflict marker.
///
/// **Empty, and that is the finding.** The plan was to exempt this file, which
/// spells the markers out, and `docs/decisions.md`, which describes the gate.
/// Neither needs it: matching only at the start of a line already separates a
/// marker from prose about one, and both spellings here sit indented inside a
/// `let`. Tested by putting a real conflict in `docs/decisions.md` and
/// watching it be caught.
///
/// Which matters, because `docs/decisions.md` is the file that conflicts on
/// *every* rebase — it is where all three of today's conflicts were. An
/// exemption list arrived at by reasoning would have excused the one file
/// most likely to carry a real marker, and the gate would have looked right.
///
/// Kept as a list rather than removed so that a document which genuinely
/// needs to open a line with `>>>>>>>` can be named here and seen, the way
/// the portability gate's exception is written where it is read.
const MARKER_EXEMPT: &[&str] = &[];

/// Fail if any tracked file carries an unresolved merge conflict.
///
/// `cargo xtask check` used to report `all gates passed` on a tree holding a
/// live `<<<<<<< HEAD`, for every file that is not Rust. The numbering check
/// reads `## N.` headings and markers do not disturb the sequence; `fmt`,
/// `clippy` and `test` never open a `.md` file. So `docs/`, the workflows,
/// the frontend, `.gitignore` and the Gradle scripts were all uncovered, and
/// a `.rs` file was covered only incidentally by ceasing to compile (#103).
///
/// Found by running the gate on a rebase that had just said `Could not
/// apply`, which is the way these are always found.
fn conflict_markers() -> anyhow::Result<()> {
    // Line-start forms only. Matching the bare strings anywhere would fire on
    // this file, on prose describing a merge, and on any diff pasted into a
    // document — a gate that cries wolf is a gate that gets worked around,
    // and a gate that is worked around is not a gate.
    //
    // `|||||||` is diff3's base section. A conflict is caught without it,
    // since the other three are always present, but leaving it out would mean
    // reporting three of a diff3 conflict's four lines — and a report that
    // silently accounts for part of what is wrong is the habit this gate
    // exists to break.
    let starts = ["<<<<<<< ", ">>>>>>> ", "||||||| "];
    let mut findings = Vec::new();

    for file in tracked_files()? {
        if MARKER_EXEMPT.iter().any(|e| file == Path::new(e)) {
            continue;
        }
        // Not every tracked file is text; a PNG that happens to hold those
        // bytes is not a conflict.
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        for (i, line) in text.lines().enumerate() {
            let bare = line == "=======" || line == "|||||||";
            if starts.iter().any(|m| line.starts_with(m)) || bare {
                findings.push(format!("  {}:{}: {}", file.display(), i + 1, line));
            }
        }
    }

    if !findings.is_empty() {
        eprintln!("unresolved conflict markers:");
        for f in &findings {
            eprintln!("{f}");
        }
        anyhow::bail!("finish the merge before running the gates");
    }
    println!("conflicts ok: no markers in tracked files");
    Ok(())
}

/// Fail if a multi-line `run:` step is anything but a literal block scalar.
///
/// YAML folds both the plain form and `>` into a single line, so a shell line
/// continuation survives as a literal backslash and the shell reads `\ ` as an
/// escaped space. The next path becomes an argument with a leading space,
/// naming a file that does not exist. In #97 that left `keystore.properties`
/// on disk with three passwords in it while the step exited 0, because
/// `rm -f` is silent by design (#102).
///
/// **The rule is `|`, not "a block scalar".** `>` folds exactly as the plain
/// form does — checked against the same input rather than read off the spec —
/// so a rule phrased against the plain form would certify the trap.
///
/// Lexical on purpose, and about the *form* rather than about backslashes.
/// The file is correct YAML and correct shell separately; the meaning is lost
/// in the handover, so the symptom is a bad thing to match on. It also has to
/// stay a rule a person can apply by eye, which matters more than the
/// automation does.
fn workflow_run_steps() -> anyhow::Result<()> {
    let mut findings = Vec::new();
    let mut checked = 0usize;
    let mut spanning = 0usize;

    for file in tracked_files()? {
        if file.parent() != Some(Path::new(".github/workflows")) {
            continue;
        }
        let text = std::fs::read_to_string(&file)?;
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let Some((before, after)) = split_run_key(line) else {
                continue;
            };
            checked += 1;
            let indent = before.len();
            let value = after.trim();
            if value.is_empty() || value.starts_with('|') {
                if !value.is_empty() {
                    spanning += 1;
                }
                // A literal block scalar, or a key whose value is on the
                // following lines — `run: |` is the only accepted spelling
                // and an empty value cannot fold anything.
                continue;
            }
            if value.starts_with('>') {
                findings.push(format!(
                    "  {}:{}: `run: >` folds newlines into spaces; use `run: |`",
                    file.display(),
                    i + 1
                ));
                continue;
            }
            // A plain scalar with its value on this line. It only folds if
            // something follows it at a deeper indent — a one-line `run:` is
            // fine and most of ours are.
            let folds = lines[i + 1..]
                .iter()
                .find(|l| !l.trim().is_empty())
                .is_some_and(|l| l.len() - l.trim_start().len() > indent);
            if folds {
                findings.push(format!(
                    "  {}:{}: a `run:` spanning lines must use `|`; this one folds",
                    file.display(),
                    i + 1
                ));
            }
        }
    }

    if !findings.is_empty() {
        eprintln!("workflow run steps that lose shell line continuations:");
        for f in &findings {
            eprintln!("{f}");
        }
        anyhow::bail!("a multi-line `run:` must use `|`");
    }
    // Both numbers, because "26 run steps, every multi-line one literal" was
    // generous about what it had inspected: the count was of every `run:`
    // key, most of them one-liners the rule does not apply to. A count exists
    // to prove the gate looked at something, so it should say at what.
    println!("workflows ok: {spanning} of {checked} run steps span lines, all literal");
    Ok(())
}

/// The indent before a `run:` key, and whatever follows it on the same line.
///
/// `None` for anything that is not the key — including `  # run: …` in a
/// comment and a `run` that is part of a longer word.
fn split_run_key(line: &str) -> Option<(&str, &str)> {
    let trimmed = line.trim_start();
    // A step's first key carries the list dash; the rest do not — and the
    // dash counts toward the key's column. Measuring the indent before it put
    // `- run:` two columns to the left of where it is, so every sibling key
    // of a *one-line* run step read as "deeper than the key", which is the
    // test for a folding plain scalar. Any step written dash-first with
    // `name:` or `if:` after it would have been flagged for a fold that is
    // not there. None exists here yet, so it passed at 26 while being wrong.
    let body = trimmed.strip_prefix("- ").unwrap_or(trimmed);
    let indent = &line[..line.len() - body.len()];
    let rest = body.strip_prefix("run:")?;
    // `run:x` is not valid YAML for a mapping key, and `running:` must not
    // match — the strip above already rules the second out, but a key with no
    // space after the colon is worth ignoring rather than guessing about.
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    let consumed = line.len() - rest.len();
    Some((indent, &line[consumed..]))
}

/// Every file git is tracking.
///
/// `git ls-files` rather than a directory walk, so generated output, build
/// directories and anything ignored are all excluded for free — and so the
/// gate's idea of "in the repository" is git's rather than a second one that
/// can drift from it.
fn tracked_files() -> anyhow::Result<Vec<PathBuf>> {
    let out = std::process::Command::new("git")
        .args(["ls-files", "-z"])
        .output()
        .context("running git ls-files")?;
    if !out.status.success() {
        anyhow::bail!("git ls-files failed");
    }
    Ok(out
        .stdout
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| PathBuf::from(String::from_utf8_lossy(s).into_owned()))
        .collect())
}

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
