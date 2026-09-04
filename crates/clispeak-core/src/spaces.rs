//! Every space this device belongs to.
//!
//! A space is a roster; belonging to several means holding several, kept
//! fully separate. Separation is the point — a work message arriving on the
//! family tablet is the failure this exists to prevent — so nothing here ever
//! merges two, and no selector means "everywhere at once".
//!
//! Spaces are never shared with another person, by design. Several spaces are
//! several sets of *your own* devices.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::roster::{Roster, RosterError};

/// One space, as this device sees it.
#[derive(Debug, Clone)]
pub struct SpaceInfo {
    /// Agreed between members, derived from the founder.
    pub id: String,
    /// This device's own name for it. Local, like a device label.
    pub label: String,
    /// How many devices are in it.
    pub devices: usize,
    /// Whether bare device names resolve here.
    pub is_default: bool,
    /// Whether this device founded it.
    pub founded_here: bool,
}

/// The spaces this device belongs to.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Spaces {
    /// Keyed by space id.
    rosters: BTreeMap<String, Roster>,
    /// This device's own name for each space.
    labels: BTreeMap<String, String>,
    /// Which space bare device names resolve in.
    default_id: String,
}

impl Spaces {
    /// The space bare names resolve in.
    pub fn default_id(&self) -> &str {
        &self.default_id
    }

    /// The roster for the default space.
    ///
    /// Every operation that predates several spaces goes through here, which
    /// is what let the rest of the node stay as it was.
    pub fn current(&self) -> &Roster {
        self.rosters
            .get(&self.default_id)
            .or_else(|| self.rosters.values().next())
            .expect("a node always has at least one space")
    }

    /// The roster for the default space, to be changed.
    pub fn current_mut(&mut self) -> &mut Roster {
        let id = self.default_id.clone();
        self.rosters.entry(id).or_default()
    }

    /// Replace the default space's roster, re-keying it if its id changed.
    ///
    /// Founding or rotating produces a roster with a different id, and
    /// leaving it under the old key would leave two entries for one space.
    pub fn replace_current(&mut self, roster: Roster) {
        let id = self.default_id.clone();
        self.replace(&id, roster);
    }

    /// Replace one named space's roster, re-keying it and keeping its label.
    ///
    /// Named rather than assumed. `replace_current` reads `default_id`, which
    /// was right while every caller meant the default — and then `rotate`
    /// began resolving a space by name and still called it, so replacing
    /// `work` destroyed `home` and left `work` untouched. The reply even
    /// listed `work`'s devices, because that was the only thing the resolved
    /// name was used for.
    ///
    /// The default only moves when the space being replaced *was* the
    /// default. Rotating a space you are not currently pointing at should not
    /// quietly repoint you at it.
    ///
    /// Returns whether the space was held at all.
    pub fn replace(&mut self, id: &str, roster: Roster) -> bool {
        if !self.rosters.contains_key(id) {
            return false;
        }
        let label = self.labels.remove(id).unwrap_or_else(|| "main".into());
        self.rosters.remove(id);

        let new_id = roster.space_id();
        self.rosters.insert(new_id.clone(), roster);
        self.labels.insert(new_id.clone(), label);
        if self.default_id == id {
            self.default_id = new_id;
        }
        true
    }

    /// Whether the default space is one nobody else ever joined.
    ///
    /// Every node founds a space for itself at first start. Joining should
    /// displace *that* — otherwise a fresh device ends up holding an
    /// abandoned empty space beside the one it just joined — and should never
    /// displace a space with other devices in it, which is a real space
    /// somebody would lose.
    pub fn current_is_unshared(&self, me: &str) -> bool {
        let roster = self.current();
        roster.members().count() <= 1 && roster.members().all(|m| m.endpoint_id == me)
    }

    /// A space by id.
    pub fn get(&self, id: &str) -> Option<&Roster> {
        self.rosters.get(id)
    }

    /// A space by id, to be changed.
    pub fn get_mut(&mut self, id: &str) -> Option<&mut Roster> {
        self.rosters.get_mut(id)
    }

    /// Which space a peer belongs to.
    ///
    /// The default is checked first, so the common case of one space is
    /// answered without scanning. Used when a peer does not say which space
    /// it means — a peer old enough to predate the field, or one that only
    /// ever shared a single space with us.
    pub fn space_of(&self, endpoint_id: &str) -> Option<String> {
        if self
            .current()
            .members()
            .any(|m| m.endpoint_id == endpoint_id)
        {
            return Some(self.default_id.clone());
        }
        self.rosters
            .iter()
            .find(|(_, r)| r.members().any(|m| m.endpoint_id == endpoint_id))
            .map(|(id, _)| id.clone())
    }

    /// Add a space, or replace one already held under the same id.
    pub fn insert(&mut self, roster: Roster, label: &str) -> String {
        let id = roster.space_id();
        self.rosters.insert(id.clone(), roster);
        self.labels.insert(id.clone(), label.to_string());
        if self.default_id.is_empty() {
            self.default_id = id.clone();
        }
        id
    }

    /// Drop a space.
    ///
    /// Refuses to drop the last one: a device with no space cannot speak even
    /// to itself, which is a state nothing else in the node expects.
    pub fn remove(&mut self, id: &str) -> Result<(), String> {
        if self.rosters.len() <= 1 {
            return Err("this is the only space; use `leave` instead".into());
        }
        if self.rosters.remove(id).is_none() {
            return Err(format!("no space with id {id}"));
        }
        self.labels.remove(id);
        if self.default_id == id {
            self.default_id = self
                .rosters
                .keys()
                .next()
                .cloned()
                .expect("at least one space remains");
        }
        Ok(())
    }

    /// Find a space by the name this device calls it.
    pub fn by_label(&self, label: &str) -> Option<String> {
        self.labels
            .iter()
            .find(|(_, l)| l.as_str() == label)
            .map(|(id, _)| id.clone())
    }

    /// This device's name for a space.
    pub fn label(&self, id: &str) -> &str {
        self.labels.get(id).map_or("main", String::as_str)
    }

    /// Rename a space locally.
    ///
    /// Labels have to be unique because they qualify device names —
    /// `work/laptop` cannot mean two things.
    pub fn set_label(&mut self, id: &str, label: &str) -> Result<(), String> {
        let label = label.trim();
        if label.is_empty() {
            return Err("a space needs a name".into());
        }
        if label.contains('/') || label.contains(',') {
            return Err("a space name cannot contain '/' or ','".into());
        }
        // Refused rather than escaped, because a label is chosen here rather
        // than arriving from a peer, and because every message below puts it
        // between quotes in prose the CLI now prints with its line breaks
        // intact (#135).
        if label.chars().any(char::is_control) {
            return Err("a space name cannot contain control characters".into());
        }
        if self.by_label(label).is_some_and(|other| other != id) {
            return Err(format!("there is already a space called '{label}'"));
        }
        if !self.rosters.contains_key(id) {
            return Err(format!("no space with id {id}"));
        }
        self.labels.insert(id.to_string(), label.to_string());
        Ok(())
    }

    /// Choose which space bare device names resolve in.
    pub fn set_default(&mut self, id: &str) -> Result<(), String> {
        if !self.rosters.contains_key(id) {
            return Err(format!("no space with id {id}"));
        }
        self.default_id = id.to_string();
        Ok(())
    }

    /// Every space, for `clispeak space list`.
    pub fn list(&self, me: &str) -> Vec<SpaceInfo> {
        self.rosters
            .iter()
            .map(|(id, roster)| SpaceInfo {
                id: id.clone(),
                label: self.label(id).to_string(),
                devices: roster.members().count(),
                is_default: *id == self.default_id,
                founded_here: id.starts_with(me),
            })
            .collect()
    }

    /// Every space's id.
    pub fn ids(&self) -> Vec<String> {
        self.rosters.keys().cloned().collect()
    }

    /// Where the spaces file lives.
    pub fn default_path() -> Result<PathBuf, RosterError> {
        crate::identity::config_dir()
            .map(|d| d.join("spaces.cbor"))
            .map_err(|e| RosterError::Storage(e.to_string()))
    }

    /// Load, migrating a single-space roster if that is all there is.
    ///
    /// The old file is left in place rather than deleted: it costs nothing,
    /// and it means downgrading to a previous build still finds its space.
    pub fn load(path: &Path, legacy: &Path) -> Result<Self, RosterError> {
        match std::fs::read(path) {
            Ok(bytes) => {
                let mut spaces: Self = ciborium::from_reader(&bytes[..])
                    .map_err(|e| RosterError::Storage(e.to_string()))?;
                // A default naming a space that is no longer held would make
                // every bare device name unresolvable.
                if !spaces.rosters.contains_key(&spaces.default_id)
                    && let Some(first) = spaces.rosters.keys().next().cloned()
                {
                    spaces.default_id = first;
                }
                Ok(spaces)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let roster = Roster::load(legacy)?;
                let mut spaces = Self::default();
                if roster.members().count() > 0 {
                    spaces.insert(roster, "main");
                }
                Ok(spaces)
            }
            Err(e) => Err(RosterError::Storage(e.to_string())),
        }
    }

    /// Write to disk.
    pub fn save(&self, path: &Path) -> Result<(), RosterError> {
        if let Some(parent) = path.parent() {
            crate::store::create_dir_private(parent)
                .map_err(|e| RosterError::Storage(e.to_string()))?;
        }
        let mut buf = Vec::new();
        ciborium::into_writer(self, &mut buf).map_err(|e| RosterError::Storage(e.to_string()))?;
        // Losing this file loses every pairing, so it is never
        // truncated in place.
        crate::store::write_private(path, &buf).map_err(|e| RosterError::Storage(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iroh_base::SecretKey;

    fn a_space(seed: u8, name: &str) -> Roster {
        let secret = SecretKey::from_bytes(&[seed; 32]);
        Roster::found(&secret, name)
    }

    #[test]
    fn the_first_space_added_becomes_the_default() {
        let mut spaces = Spaces::default();
        let id = spaces.insert(a_space(1, "laptop"), "home");
        assert_eq!(spaces.default_id(), id);
        assert_eq!(spaces.label(&id), "home");
    }

    #[test]
    fn two_spaces_founded_by_one_device_are_still_two_spaces() {
        // Rotating produces exactly this: same founder, different space.
        let mut spaces = Spaces::default();
        let first = spaces.insert(a_space(1, "laptop"), "home");
        let mut second = a_space(1, "laptop");
        // Force a different founding moment, which is what distinguishes them.
        second = Roster::from_parts(
            second
                .members()
                .map(|m| {
                    let mut m = m.clone();
                    m.joined_at += 1;
                    m
                })
                .collect(),
            Vec::new(),
        )
        .0;
        let second = spaces.insert(second, "work");
        assert_ne!(first, second);
        assert_eq!(spaces.list("").len(), 2);
    }

    #[test]
    fn labels_have_to_be_unique_because_they_qualify_device_names() {
        let mut spaces = Spaces::default();
        let first = spaces.insert(a_space(1, "laptop"), "home");
        let mut other = a_space(2, "desk");
        other = Roster::from_parts(other.members().cloned().collect(), Vec::new()).0;
        let second = spaces.insert(other, "work");

        assert!(spaces.set_label(&second, "home").is_err());
        // Renaming a space to what it is already called is not a clash.
        assert!(spaces.set_label(&first, "home").is_ok());
        // `/` and `,` would break the selector syntax outright.
        assert!(spaces.set_label(&first, "home/again").is_err());
        assert!(spaces.set_label(&first, "a,b").is_err());
        assert!(spaces.set_label(&first, "  ").is_err());
    }

    #[test]
    fn a_space_with_other_devices_in_it_is_never_displaced_by_joining() {
        // The bug this guards: `do_join` replaced the current space, so a
        // device in `home` that joined `work` lost `home` entirely.
        let mut spaces = Spaces::default();
        spaces.insert(a_space(1, "laptop"), "home");
        let me = SecretKey::from_bytes(&[1; 32]).public().to_string();

        // Alone: the empty space a node founds for itself, safe to displace.
        assert!(spaces.current_is_unshared(&me));

        // Once somebody else is in it, it is a real space.
        let mut shared = a_space(1, "laptop");
        let other = SecretKey::from_bytes(&[9; 32]);
        shared.invite(
            &SecretKey::from_bytes(&[1; 32]),
            &other.public().to_string(),
            "phone",
        );
        spaces.replace_current(shared);
        assert!(!spaces.current_is_unshared(&me));

        // And it is not this device's own membership that makes it shared.
        assert!(!spaces.current_is_unshared("somebody-else"));
    }

    #[test]
    fn replacing_a_named_space_leaves_every_other_one_alone() {
        // The bug this guards was destructive and live: `rotate` resolved the
        // space by name and then called `replace_current`, which reads
        // `default_id` — so replacing `work` destroyed `home`, moved `home`'s
        // label onto the fresh empty roster, and left `work` and all its keys
        // exactly where they were. Every existing test only ever exercised
        // the default, which is why it went unnoticed.
        let mut spaces = Spaces::default();
        let home = spaces.insert(a_space(1, "laptop"), "home");
        let work = spaces.insert(a_space(2, "desk"), "work");
        spaces.set_default(&home).expect("default");

        assert!(spaces.replace(&work, a_space(3, "desk")));

        assert!(spaces.get(&home).is_some(), "the default must be untouched");
        assert_eq!(spaces.default_id(), home, "and must still be the default");
        assert!(spaces.get(&work).is_none(), "the named space is replaced");
        assert!(spaces.by_label("work").is_some(), "keeping its label");
        assert_eq!(spaces.by_label("home"), Some(home.clone()));
    }

    #[test]
    fn replacing_the_default_moves_the_default_with_it() {
        let mut spaces = Spaces::default();
        let home = spaces.insert(a_space(1, "laptop"), "home");
        spaces.insert(a_space(2, "desk"), "work");
        spaces.set_default(&home).expect("default");

        spaces.replace(&home, a_space(3, "laptop"));

        assert_ne!(spaces.default_id(), home);
        assert_eq!(spaces.label(spaces.default_id()), "home");
    }

    #[test]
    fn replacing_a_space_that_is_not_held_changes_nothing() {
        let mut spaces = Spaces::default();
        let home = spaces.insert(a_space(1, "laptop"), "home");
        assert!(!spaces.replace("no-such-space", a_space(2, "desk")));
        assert_eq!(spaces.ids(), vec![home]);
    }

    #[test]
    fn the_last_space_cannot_be_dropped() {
        let mut spaces = Spaces::default();
        let only = spaces.insert(a_space(1, "laptop"), "home");
        // A device with no space cannot speak even to itself.
        assert!(spaces.remove(&only).is_err());
    }

    #[test]
    fn dropping_the_default_promotes_another() {
        let mut spaces = Spaces::default();
        let first = spaces.insert(a_space(1, "laptop"), "home");
        let second = spaces.insert(a_space(2, "desk"), "work");
        spaces.set_default(&second).expect("set default");

        spaces.remove(&second).expect("remove");
        assert_eq!(spaces.default_id(), first);
        assert!(spaces.get(&second).is_none());
    }

    #[test]
    fn a_peer_is_found_in_the_space_it_belongs_to() {
        let mut spaces = Spaces::default();
        let home = spaces.insert(a_space(1, "laptop"), "home");
        let work = spaces.insert(a_space(2, "desk"), "work");

        let laptop = SecretKey::from_bytes(&[1; 32]).public().to_string();
        let desk = SecretKey::from_bytes(&[2; 32]).public().to_string();
        assert_eq!(spaces.space_of(&laptop), Some(home));
        assert_eq!(spaces.space_of(&desk), Some(work));
        assert_eq!(spaces.space_of("someone-else"), None);
    }

    #[test]
    fn replacing_the_current_space_rekeys_it_and_keeps_its_name() {
        // What rotating does: a new space under the old local label.
        let mut spaces = Spaces::default();
        let before = spaces.insert(a_space(1, "laptop"), "home");
        spaces.replace_current(a_space(2, "laptop"));

        assert_ne!(spaces.default_id(), before);
        assert!(spaces.get(&before).is_none());
        assert_eq!(spaces.label(spaces.default_id()), "home");
        assert_eq!(spaces.ids().len(), 1);
    }
}
