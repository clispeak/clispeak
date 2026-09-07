//! Installing the agent skill.
//!
//! The skill is compiled in rather than read from disk, so `clispeak skill`
//! works from a single binary with nothing beside it — which is the whole
//! point of a tool an agent installs and calls.
//!
//! **Installing lives here rather than in the app** because the app may be
//! sandboxed and the command line tool never is. Inside a Flatpak an
//! unshared write appears to succeed and never reaches the host, so an app
//! that offered to install anywhere would report success and install nothing.
//! This runs as the user, with the user's filesystem, and can write wherever
//! they say.

use std::path::{Path, PathBuf};

/// The skill itself, from the file the drift test checks.
pub const SKILL: &str = include_str!("../../../skills/clispeak/SKILL.md");

/// Where Claude Code looks for skills.
///
/// Only a default. Agents that keep skills elsewhere are the reason `--to`
/// exists, and the reason this is a suggestion rather than a fixed location.
pub fn default_destination() -> Option<PathBuf> {
    Some(
        directories::BaseDirs::new()?
            .home_dir()
            .join(".claude/skills/clispeak/SKILL.md"),
    )
}

/// Whether an installed copy is missing or out of date.
///
/// Compared by content, like the CLI install the app already does: the
/// question is whether the file says what this build says, and a version
/// string would only be a proxy for that.
pub fn state(path: &Path) -> State {
    match std::fs::read_to_string(path) {
        Ok(text) if text == SKILL => State::Current,
        Ok(_) => State::Stale,
        Err(_) => State::Absent,
    }
}

/// What was found at a destination.
#[derive(Debug, PartialEq, Eq)]
pub enum State {
    /// Nothing there.
    Absent,
    /// There, and matching this build.
    Current,
    /// There, and saying something else.
    Stale,
}

/// Expand a leading `~`, which a shell would have done and a quoted path
/// prevents.
///
/// Most tools leave this to the shell, and rightly: `cat "~/x"` failing is a
/// clear error. Here it was not. A destination path is *created*, so
/// `--path "~/.claude/..."` made a directory literally named `~` in whatever
/// directory the command ran from, wrote the skill inside it, and reported
/// success — the file nowhere an agent looks, and nothing to say so.
///
/// That is the same silent-success failure that put installing in this binary
/// rather than the sandboxed app, reappearing one level down. An agent
/// composing this call is more likely to quote the path than a person is.
pub fn expand_home(path: &Path) -> PathBuf {
    let Ok(rest) = path.strip_prefix("~") else {
        return path.to_path_buf();
    };
    match directories::BaseDirs::new() {
        Some(dirs) => dirs.home_dir().join(rest),
        // No home to expand to. Left alone so the write fails loudly rather
        // than landing somewhere unintended.
        None => path.to_path_buf(),
    }
}

/// Write the skill, creating the directory if needed.
pub fn install(path: &Path) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, SKILL)
}

/// Where Claude Code keeps the settings that hold hooks.
pub fn settings_path() -> Option<PathBuf> {
    Some(
        directories::BaseDirs::new()?
            .home_dir()
            .join(".claude/settings.json"),
    )
}

/// What the hook runs. One line, on stdout, into the model's context.
const HOOK_COMMAND: &str = "clispeak prefs --brief";

/// Install the hook that keeps the agreement in front of an agent.
///
/// **This is the part that survives compaction, and nothing else does.** An
/// agent reads the agreement once and then the conversation is summarised;
/// the text is gone and the agent carries on without it. A `UserPromptSubmit`
/// hook writes one line into context on *every* prompt, so what was
/// summarised away is replaced before the next turn — there is nothing to
/// remember and nothing to lose (#231).
///
/// Written into `settings.json` rather than the skill's own frontmatter. A
/// skill can carry hooks and that would be tidier, since removing the skill
/// would remove the hook by construction — but the exact frontmatter shape
/// for a *command* hook is not something to guess at when this is the
/// mechanism the whole design rests on. `forget` removes this explicitly
/// instead.
pub fn install_hook() -> std::io::Result<PathBuf> {
    let path = settings_path()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no home directory"))?;
    let mut settings = read_settings(&path);
    let entries = hook_entries(&mut settings);
    if entries.iter().any(is_ours) {
        return Ok(path);
    }
    entries.push(serde_json::json!({
        "hooks": [{ "type": "command", "command": HOOK_COMMAND }]
    }));
    write_settings(&path, &settings)?;
    Ok(path)
}

/// Take the hook out again, leaving anything else in the file alone.
pub fn remove_hook() -> std::io::Result<bool> {
    let Some(path) = settings_path() else {
        return Ok(false);
    };
    if !path.exists() {
        return Ok(false);
    }
    let mut settings = read_settings(&path);
    let entries = hook_entries(&mut settings);
    let before = entries.len();
    entries.retain(|e| !is_ours(e));
    let removed = entries.len() != before;
    // Tidy up after ourselves: an empty array left behind is a change to
    // somebody's settings file that says nothing and was not asked for.
    if entries.is_empty()
        && let Some(hooks) = settings.get_mut("hooks").and_then(|h| h.as_object_mut())
    {
        hooks.remove("UserPromptSubmit");
        if hooks.is_empty()
            && let Some(root) = settings.as_object_mut()
        {
            root.remove("hooks");
        }
    }
    if removed {
        write_settings(&path, &settings)?;
    }
    Ok(removed)
}

/// Whether the hook is installed.
pub fn hook_installed() -> bool {
    let Some(path) = settings_path() else {
        return false;
    };
    let mut settings = read_settings(&path);
    hook_entries(&mut settings).iter().any(is_ours)
}

/// Ours rather than somebody else's, by what it runs.
fn is_ours(entry: &serde_json::Value) -> bool {
    entry["hooks"]
        .as_array()
        .is_some_and(|hs| hs.iter().any(|h| h["command"] == HOOK_COMMAND))
}

/// A parsed settings file, or an empty one.
///
/// Unreadable or malformed reads as empty rather than as an error — but the
/// write below will then replace it, so this is only safe because a
/// *malformed* settings file is one Claude Code is not reading either.
fn read_settings(path: &Path) -> serde_json::Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| serde_json::json!({}))
}

/// The `hooks.UserPromptSubmit` array, created if absent.
fn hook_entries(settings: &mut serde_json::Value) -> &mut Vec<serde_json::Value> {
    settings
        .as_object_mut()
        .expect("settings is an object")
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .expect("hooks is an object")
        .entry("UserPromptSubmit")
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .expect("UserPromptSubmit is an array")
}

/// Write it back, pretty, atomically.
///
/// Atomic because this is somebody's editor configuration and a half-written
/// one stops their agent starting. Same reasoning as the config file.
fn write_settings(path: &Path, settings: &serde_json::Value) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    std::fs::write(&tmp, serde_json::to_string_pretty(settings)?.as_bytes())?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_compiled_skill_is_the_file_on_disk() {
        // include_str! guarantees this at compile time; the test states it so
        // the guarantee is visible to someone reading the module.
        assert!(SKILL.contains("this is"), "the identification rule is gone");
        assert!(SKILL.starts_with("---"), "front matter is missing");
    }

    #[test]
    fn a_quoted_home_path_expands_instead_of_making_a_directory_called_tilde() {
        let home = directories::BaseDirs::new().expect("a home directory");
        let expanded = expand_home(Path::new("~/.claude/skills/clispeak/SKILL.md"));

        assert_eq!(
            expanded,
            home.home_dir().join(".claude/skills/clispeak/SKILL.md")
        );
        assert!(
            !expanded.starts_with("~"),
            "a leading tilde must not survive into a path that gets created"
        );
        // Bare `~` is the home directory itself.
        assert_eq!(expand_home(Path::new("~")), home.home_dir());
        // Anything else is left exactly as given.
        assert_eq!(expand_home(Path::new("/tmp/x")), Path::new("/tmp/x"));
        assert_eq!(
            expand_home(Path::new("relative/x")),
            Path::new("relative/x")
        );
        // A tilde that is not the first component is a real directory name.
        assert_eq!(expand_home(Path::new("/tmp/~/x")), Path::new("/tmp/~/x"));
    }

    #[test]
    fn a_missing_file_is_absent_and_a_written_one_is_current() {
        let dir = std::env::temp_dir().join(format!("clispeak-skill-{}", std::process::id()));
        let path = dir.join("SKILL.md");
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(state(&path), State::Absent);
        install(&path).expect("install");
        assert_eq!(state(&path), State::Current);

        std::fs::write(&path, "something else").expect("overwrite");
        assert_eq!(state(&path), State::Stale);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
