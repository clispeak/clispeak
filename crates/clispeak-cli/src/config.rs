//! Sender-side configuration: defaults and groups.
//!
//! Read by the CLI, never by the node. Groups in particular are deliberately
//! local — they expand to device names before a request is sent, so they
//! never appear in the protocol and two devices need not agree on what
//! "phones" means.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Deserialize;

/// What `~/.config/clispeak/config.toml` holds.
#[derive(Debug, Default, Deserialize)]
pub struct Config {
    /// Where messages go when `--to` is not given.
    #[serde(default)]
    pub default_target: Option<String>,
    /// Urgency when `--priority` is not given.
    #[serde(default)]
    pub default_priority: Option<String>,
    /// Named sets of devices.
    #[serde(default)]
    pub groups: BTreeMap<String, Vec<String>>,
    /// How this user wants to be spoken to. See [`crate::prefs`].
    #[serde(default)]
    pub agent: Option<crate::prefs::Agreement>,
}

/// Where the config lives.
///
/// Mirrors `clispeak_core::config_dir`, including the environment override
/// that lets a second node run alongside the first. The CLI deliberately does
/// not depend on the node's crate — see the socket name for the same trade —
/// so this has to stay in step by hand.
pub fn path() -> Option<PathBuf> {
    dir().map(|d| d.join("config.toml"))
}

/// The directory the node keeps its state in, resolved the same way the node
/// resolves it.
///
/// The node's own copy is `clispeak_core::identity::config_dir`, which also
/// honours a path the host set — only used on mobile, where there is no CLI.
/// Kept in step by hand, like the socket name and the frame format.
pub fn dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("CLISPEAK_CONFIG_DIR") {
        return Some(PathBuf::from(dir));
    }
    directories::ProjectDirs::from("", "", "clispeak").map(|d| d.config_dir().to_path_buf())
}

/// Load the config, treating anything unreadable as empty.
///
/// A malformed file must not stop someone speaking: the tool still works with
/// no config at all, so the failure is reported and then ignored.
pub fn load() -> Config {
    let Some(p) = path() else {
        return Config::default();
    };
    let Ok(text) = std::fs::read_to_string(&p) else {
        return Config::default();
    };
    match toml::from_str(&text) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("warning: ignoring {}: {e}", p.display());
            Config::default()
        }
    }
}

/// Rewrite the `[groups]` table, leaving every other key as it was.
///
/// Read-modify-write on the parsed document rather than serialising a typed
/// struct, so settings this version has never heard of survive being edited
/// by it. Comments do not survive, which is the honest limit of doing this
/// without a format-preserving parser.
pub fn write_groups(groups: &BTreeMap<String, Vec<String>>) -> anyhow::Result<PathBuf> {
    amend(|doc| {
        if groups.is_empty() {
            doc.remove("groups");
        } else {
            let table: toml::Table = groups
                .iter()
                .map(|(name, devices)| {
                    let list = devices
                        .iter()
                        .map(|d| toml::Value::String(d.clone()))
                        .collect();
                    (name.clone(), toml::Value::Array(list))
                })
                .collect();
            doc.insert("groups".into(), toml::Value::Table(table));
        }
        Ok(())
    })
}

/// Remove the `[agent]` table, leaving every other key as it was.
pub fn remove_agent() -> anyhow::Result<PathBuf> {
    amend(|doc| {
        doc.remove("agent");
        Ok(())
    })
}

/// Read the file, change one part of it, and put it back atomically.
///
/// **Atomically, which the first version was not.** A plain write truncates
/// and then fills, so an interrupted one leaves a file that is neither the
/// old contents nor the new — and this file now holds the working agreement,
/// which nobody has a copy of anywhere else. The node's own state has been
/// written this way since #56; the config was left behind because one person
/// edits groups rarely. Several agents writing preferences makes it ordinary.
///
/// The read happens here rather than in the caller for the same reason. Two
/// agents recording a rule in the same moment would otherwise lose one, which
/// is the shape of the two bugs this project found on 7 September.
pub(crate) fn amend(
    change: impl FnOnce(&mut toml::Table) -> anyhow::Result<()>,
) -> anyhow::Result<PathBuf> {
    let p = path().ok_or_else(|| anyhow::anyhow!("no config directory on this platform"))?;
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }

    // Held across the read *and* the write. Re-reading immediately before
    // writing is not enough and this was measured, not reasoned: twenty
    // concurrent `prefs add` calls recorded fourteen rules and silently lost
    // six. Two agents writing in the same second is the ordinary case under
    // this design, so a lost write is a preference the user stated and the
    // machine forgot — the exact failure the whole change exists to end.
    let _lock = Lock::take(&p)?;

    let mut doc: toml::Table = std::fs::read_to_string(&p)
        .ok()
        .and_then(|t| t.parse().ok())
        .unwrap_or_default();
    change(&mut doc)?;

    let text = toml::to_string_pretty(&doc)?;
    // A sibling, so the rename below cannot cross a filesystem boundary. The
    // process id keeps two agents writing at once from sharing a temporary.
    let tmp = p.with_extension(format!("toml.{}.tmp", std::process::id()));
    std::fs::write(&tmp, text.as_bytes())?;
    std::fs::rename(&tmp, &p)?;
    Ok(p)
}

/// Exclusive access to the config file, for as long as this value lives.
///
/// `create_new` is the portable primitive here: it is atomic on every
/// platform this ships to, needs no `unsafe`, and this workspace forbids
/// `unsafe`. `flock` would be tidier on Unix and has no Windows twin that can
/// be reached without Win32 calls.
struct Lock(PathBuf);

impl Lock {
    /// How long to keep trying before giving up.
    const PATIENCE: std::time::Duration = std::time::Duration::from_secs(3);
    /// A lock older than this is treated as abandoned.
    ///
    /// A process killed between taking the lock and releasing it would
    /// otherwise wedge every later write forever, and the thing wedged is the
    /// file holding preferences nobody has another copy of. Ten seconds is
    /// far longer than a write of a few hundred bytes can honestly take and
    /// short enough that nobody waits on a dead process.
    const STALE: std::time::Duration = std::time::Duration::from_secs(10);

    fn take(config: &std::path::Path) -> anyhow::Result<Self> {
        let path = config.with_extension("toml.lock");
        let deadline = std::time::Instant::now() + Self::PATIENCE;
        loop {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(_) => return Ok(Self(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    if Self::abandoned(&path) {
                        let _ = std::fs::remove_file(&path);
                        continue;
                    }
                    if std::time::Instant::now() >= deadline {
                        anyhow::bail!(
                            "another clispeak has held {} for {}s. Delete it if \
                             nothing is running",
                            path.display(),
                            Self::PATIENCE.as_secs()
                        );
                    }
                    std::thread::sleep(std::time::Duration::from_millis(15));
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    fn abandoned(path: &std::path::Path) -> bool {
        std::fs::metadata(path)
            .and_then(|m| m.modified())
            .and_then(|t| t.elapsed().map_err(std::io::Error::other))
            .is_ok_and(|age| age > Self::STALE)
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Expand any group names in a selector into the devices they stand for.
///
/// One level deep: a group naming another group is left alone rather than
/// followed. Nesting buys little here and a cycle would hang the tool, which
/// is a poor trade for a convenience.
///
/// Unknown names pass through untouched — the node owns the roster, so it is
/// the only thing that can tell a typo from a device this CLI has not heard
/// of yet, and it reports one properly.
pub fn expand(selector: &str, groups: &BTreeMap<String, Vec<String>>) -> String {
    let mut out: Vec<String> = Vec::new();
    for raw in selector.split(',') {
        let name = raw.trim();
        if name.is_empty() {
            continue;
        }
        match groups.get(name) {
            Some(devices) => out.extend(devices.iter().map(|d| d.trim().to_string())),
            None => out.push(name.to_string()),
        }
    }
    out.join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn groups() -> BTreeMap<String, Vec<String>> {
        BTreeMap::from([
            ("phones".to_string(), vec!["pixel".into(), "iphone".into()]),
            ("loud".to_string(), vec!["desk".into()]),
        ])
    }

    #[test]
    fn a_group_becomes_its_devices() {
        assert_eq!(expand("phones", &groups()), "pixel,iphone");
    }

    #[test]
    fn groups_and_devices_mix_freely() {
        assert_eq!(expand("phones,desk", &groups()), "pixel,iphone,desk");
        assert_eq!(expand("loud,phones", &groups()), "desk,pixel,iphone");
    }

    #[test]
    fn unknown_names_pass_through_for_the_node_to_judge() {
        assert_eq!(expand("laptop", &groups()), "laptop");
        assert_eq!(expand("all", &groups()), "all");
    }

    #[test]
    fn whitespace_and_empty_elements_are_tidied_away() {
        assert_eq!(expand(" phones , desk , ", &groups()), "pixel,iphone,desk");
    }

    #[test]
    fn a_group_naming_a_group_is_not_followed() {
        let nested = BTreeMap::from([
            ("outer".to_string(), vec!["inner".into()]),
            ("inner".to_string(), vec!["desk".into()]),
        ]);
        assert_eq!(expand("outer", &nested), "inner");
    }
}
