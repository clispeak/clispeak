//! Sender-side configuration: defaults and groups.
//!
//! Read by the CLI, never by the node. Groups in particular are deliberately
//! local — they expand to device names before a request is sent, so they
//! never appear in the protocol and two devices need not agree on what
//! "phones" means.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Deserialize;

/// What `~/.config/voicecast/config.toml` holds.
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
}

/// Where the config lives.
///
/// Mirrors `voicecast_core::config_dir`, including the environment override
/// that lets a second node run alongside the first. The CLI deliberately does
/// not depend on the node's crate — see the socket name for the same trade —
/// so this has to stay in step by hand.
pub fn path() -> Option<PathBuf> {
    dir().map(|d| d.join("config.toml"))
}

/// The directory the node keeps its state in, resolved the same way the node
/// resolves it.
///
/// The node's own copy is `voicecast_core::identity::config_dir`, which also
/// honours a path the host set — only used on mobile, where there is no CLI.
/// Kept in step by hand, like the socket name and the frame format.
pub fn dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("VOICECAST_CONFIG_DIR") {
        return Some(PathBuf::from(dir));
    }
    directories::ProjectDirs::from("", "", "voicecast").map(|d| d.config_dir().to_path_buf())
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
    let p = path().ok_or_else(|| anyhow::anyhow!("no config directory on this platform"))?;
    let mut doc: toml::Table = std::fs::read_to_string(&p)
        .ok()
        .and_then(|t| t.parse().ok())
        .unwrap_or_default();

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

    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&p, toml::to_string_pretty(&doc)?)?;
    Ok(p)
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
