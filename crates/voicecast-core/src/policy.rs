//! What this device will and will not say out loud.
//!
//! Receiver-side, deliberately: the sender states urgency, the device that
//! actually makes noise decides whether noise is acceptable right now. A
//! sender cannot mute or unmute anyone else, which is the whole point — an
//! agent marking everything urgent must not be able to wake the house.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use voicecast_proto::{Priority, Status};

/// How many messages already waiting counts as "deep".
///
/// `low` is for chatter, and chatter that has fallen this far behind is stale
/// by the time it would be heard. Small on purpose: a device speaks one
/// message at a time, and each one takes seconds, so five is already a minute
/// or more of backlog.
const DEEP_QUEUE: usize = 5;

/// A daily window during which this device stays quiet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuietHours {
    /// Start, as minutes past local midnight.
    pub from: u16,
    /// End, as minutes past local midnight. May be *before* `from`, which is
    /// the normal case — quiet hours usually cross midnight.
    pub to: u16,
    /// Whether `high` may break through anyway.
    ///
    /// Off by default, and that default matters: "urgent" stops meaning
    /// anything the first time an agent marks every message urgent, so
    /// breaking through has to be something the person at the device chose.
    #[serde(default)]
    pub high_breaks_through: bool,
}

impl QuietHours {
    /// Whether `minute` (past local midnight) falls inside the window.
    ///
    /// A window whose ends are equal is empty rather than eternal. Reading
    /// `22:00-22:00` as "always quiet" would silence a device permanently
    /// from what looks like a typo.
    pub fn contains(&self, minute: u16) -> bool {
        if self.from == self.to {
            return false;
        }
        if self.from < self.to {
            minute >= self.from && minute < self.to
        } else {
            // Crosses midnight: inside if after the start *or* before the end.
            minute >= self.from || minute < self.to
        }
    }
}

/// This device's speaking policy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Policy {
    /// Silenced indefinitely, until someone unmutes it.
    #[serde(default)]
    pub muted: bool,
    /// The daily quiet window, if one is set.
    #[serde(default)]
    pub quiet: Option<QuietHours>,
}

impl Policy {
    /// Why this message will not be spoken, or `None` to go ahead.
    ///
    /// Returns a [`Status`] rather than a bool so the sender is told *which*
    /// policy stopped it — "muted" and "quiet hours" call for different
    /// reactions, and a bare failure invites a retry that will fail the same
    /// way.
    pub fn verdict(&self, priority: Priority, minute: u16, queued: usize) -> Option<Status> {
        // Mute is unconditional. Nothing breaks through it, including
        // `high` — a muted device is one someone deliberately silenced.
        if self.muted {
            return Some(Status::Muted);
        }
        if let Some(q) = self.quiet
            && q.contains(minute)
        {
            let allowed = priority == Priority::High && q.high_breaks_through;
            if !allowed {
                return Some(Status::QuietHours);
            }
        }
        // Chatter that has queued up behind real messages is stale by the
        // time it would be heard.
        if priority == Priority::Low && queued >= DEEP_QUEUE {
            return Some(Status::Dropped);
        }
        None
    }
}

/// This device's policy, and any per-space overrides on top of it.
///
/// The device policy is a floor, not a default: an override can only make a
/// space *quieter*, never louder. Mute means mute — a space that could undo it
/// would turn the one switch everybody understands into a switch that works
/// most of the time, which is worse than not having per-space settings at all.
/// The cost is stated in `docs/decisions.md` #29: there is no way to let one
/// space through while the device is muted, and `high` remains the mechanism
/// for "reach me anyway".
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Policies {
    /// Applies to everything this device speaks.
    ///
    /// Flattened so a `policy.json` written before spaces existed still loads:
    /// its `muted` and `quiet` keys land here and `spaces` comes up empty.
    #[serde(flatten)]
    pub device: Policy,
    /// Extra restrictions for individual spaces, keyed by space id.
    ///
    /// A space with nothing to say is absent rather than present-and-empty, so
    /// "has an override" is a question the map itself answers.
    #[serde(default)]
    pub spaces: BTreeMap<String, Policy>,
}

impl Policies {
    /// Why a message arriving in `space` will not be spoken, or `None`.
    ///
    /// Both policies get a say and either can refuse, which is what makes the
    /// device policy a floor. The device is asked first so its reason is the
    /// one reported: someone who muted the whole device wants to hear "muted",
    /// not the name of whichever space the message happened to arrive in.
    pub fn verdict(
        &self,
        space: Option<&str>,
        priority: Priority,
        minute: u16,
        queued: usize,
    ) -> Option<Status> {
        if let Some(status) = self.device.verdict(priority, minute, queued) {
            return Some(status);
        }
        let over = space.and_then(|id| self.spaces.get(id))?;
        over.verdict(priority, minute, queued)
    }

    /// The override for `space`, if it has one.
    pub fn space(&self, space: &str) -> Option<&Policy> {
        self.spaces.get(space)
    }

    /// Set a space's override, dropping it entirely when it restricts nothing.
    ///
    /// Pruning matters for more than tidiness: an override that says "not
    /// muted, no quiet hours" is indistinguishable in effect from having none,
    /// and keeping it would make the interface report a space as configured
    /// when nothing about it differs.
    pub fn set_space(&mut self, space: &str, policy: Policy) {
        if policy == Policy::default() {
            self.spaces.remove(space);
        } else {
            self.spaces.insert(space.to_string(), policy);
        }
    }

    /// Forget a space's override, as when the space itself is gone.
    ///
    /// Left behind, it would silently apply again to a *new* space that
    /// happened to be founded with the same id — which rotating produces.
    pub fn forget(&mut self, space: &str) {
        self.spaces.remove(space);
    }
}

/// Minutes past local midnight, right now.
///
/// Local rather than UTC because quiet hours are about when the person is
/// asleep, which no other clock knows.
pub fn local_minute() -> u16 {
    use chrono::Timelike;
    let now = chrono::Local::now();
    (now.hour() * 60 + now.minute()) as u16
}

/// Parse `HH:MM`, the form quiet hours are written in.
pub fn parse_time(s: &str) -> Option<u16> {
    let (h, m) = s.trim().split_once(':')?;
    let h: u16 = h.trim().parse().ok()?;
    let m: u16 = m.trim().parse().ok()?;
    if h > 23 || m > 59 {
        return None;
    }
    Some(h * 60 + m)
}

/// Render minutes past midnight as `HH:MM`.
pub fn format_time(minute: u16) -> String {
    format!("{:02}:{:02}", (minute / 60) % 24, minute % 60)
}

/// Where the policy is kept.
fn path() -> Result<std::path::PathBuf, crate::IdentityError> {
    Ok(crate::config_dir()?.join("policy.json"))
}

/// Load this device's policies, falling back to "say everything".
///
/// A missing or unreadable file is not an error: a device that has never been
/// configured should speak, not sit silently for a reason nobody can see.
pub fn load() -> Policies {
    path()
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// Persist the policies.
pub fn save(policy: &Policies) -> Result<(), crate::IdentityError> {
    let p = path()?;
    if let Some(dir) = p.parent() {
        crate::store::create_dir_private(dir)
            .map_err(|e| crate::IdentityError::Store(e.to_string()))?;
    }
    let text = serde_json::to_string_pretty(policy)
        .map_err(|e| crate::IdentityError::Store(e.to_string()))?;
    // A truncated policy reads as 'nothing configured', so a muted
    // device would quietly un-mute itself.
    crate::store::write_private(&p, text.as_bytes())
        .map_err(|e| crate::IdentityError::Store(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minutes in a day, the unit quiet hours are expressed in.
    const DAY: u16 = 24 * 60;

    fn quiet(from: &str, to: &str, breaks: bool) -> QuietHours {
        QuietHours {
            from: parse_time(from).unwrap(),
            to: parse_time(to).unwrap(),
            high_breaks_through: breaks,
        }
    }

    #[test]
    fn window_crossing_midnight_covers_both_sides() {
        let q = quiet("22:00", "07:00", false);
        assert!(q.contains(parse_time("23:30").unwrap()));
        assert!(q.contains(parse_time("02:00").unwrap()));
        assert!(q.contains(parse_time("22:00").unwrap()));
        assert!(!q.contains(parse_time("07:00").unwrap()));
        assert!(!q.contains(parse_time("12:00").unwrap()));
    }

    #[test]
    fn window_within_one_day_is_half_open() {
        let q = quiet("09:00", "17:00", false);
        assert!(q.contains(parse_time("09:00").unwrap()));
        assert!(q.contains(parse_time("16:59").unwrap()));
        assert!(!q.contains(parse_time("17:00").unwrap()));
        assert!(!q.contains(parse_time("08:59").unwrap()));
    }

    #[test]
    fn an_empty_window_silences_nothing() {
        let q = quiet("22:00", "22:00", false);
        for m in 0..DAY {
            assert!(!q.contains(m), "minute {m} should not be quiet");
        }
    }

    #[test]
    fn mute_beats_every_priority() {
        let p = Policy {
            muted: true,
            quiet: None,
        };
        for prio in [Priority::Low, Priority::Normal, Priority::High] {
            assert_eq!(p.verdict(prio, 0, 0), Some(Status::Muted));
        }
    }

    #[test]
    fn high_breaks_quiet_hours_only_when_allowed() {
        let refusing = Policy {
            muted: false,
            quiet: Some(quiet("22:00", "07:00", false)),
        };
        let allowing = Policy {
            muted: false,
            quiet: Some(quiet("22:00", "07:00", true)),
        };
        let night = parse_time("23:00").unwrap();
        assert_eq!(
            refusing.verdict(Priority::High, night, 0),
            Some(Status::QuietHours)
        );
        assert_eq!(allowing.verdict(Priority::High, night, 0), None);
        // Breaking through is for `high` alone; normal traffic still waits.
        assert_eq!(
            allowing.verdict(Priority::Normal, night, 0),
            Some(Status::QuietHours)
        );
    }

    #[test]
    fn low_is_dropped_behind_a_deep_queue_but_normal_is_not() {
        let p = Policy::default();
        assert_eq!(
            p.verdict(Priority::Low, 0, DEEP_QUEUE),
            Some(Status::Dropped)
        );
        assert_eq!(p.verdict(Priority::Low, 0, DEEP_QUEUE - 1), None);
        assert_eq!(p.verdict(Priority::Normal, 0, DEEP_QUEUE + 10), None);
    }

    #[test]
    fn nothing_is_refused_by_default() {
        let p = Policy::default();
        assert_eq!(p.verdict(Priority::Normal, 0, 0), None);
        assert_eq!(p.verdict(Priority::High, 720, 0), None);
    }

    /// A space with no override behaves exactly as the device does.
    #[test]
    fn a_space_without_an_override_follows_the_device() {
        let mut ps = Policies::default();
        ps.device.quiet = Some(quiet("22:00", "07:00", false));
        let night = parse_time("23:00").unwrap();
        let noon = parse_time("12:00").unwrap();
        assert_eq!(
            ps.verdict(Some("work"), Priority::Normal, night, 0),
            Some(Status::QuietHours)
        );
        assert_eq!(ps.verdict(Some("work"), Priority::Normal, noon, 0), None);
        // And an unknown space id is the same as none at all.
        assert_eq!(ps.verdict(None, Priority::Normal, noon, 0), None);
    }

    /// The case the feature exists for: one space quiet while another speaks.
    #[test]
    fn an_override_silences_only_its_own_space() {
        let mut ps = Policies::default();
        ps.set_space(
            "work",
            Policy {
                muted: false,
                quiet: Some(quiet("18:00", "09:00", false)),
            },
        );
        let evening = parse_time("20:00").unwrap();
        assert_eq!(
            ps.verdict(Some("work"), Priority::Normal, evening, 0),
            Some(Status::QuietHours)
        );
        assert_eq!(ps.verdict(Some("home"), Priority::Normal, evening, 0), None);
        assert_eq!(ps.verdict(None, Priority::Normal, evening, 0), None);
    }

    /// The floor. An override may add silence and never remove it.
    #[test]
    fn a_space_cannot_undo_the_device_policy() {
        let mut ps = Policies::default();
        ps.device.muted = true;
        // As permissive an override as can be written.
        ps.set_space(
            "home",
            Policy {
                muted: false,
                quiet: Some(quiet("00:00", "00:00", true)),
            },
        );
        for prio in [Priority::Low, Priority::Normal, Priority::High] {
            assert_eq!(
                ps.verdict(Some("home"), prio, 720, 0),
                Some(Status::Muted),
                "a space override must not speak through a muted device"
            );
        }
    }

    /// Muting one space leaves the device — and every other space — alone.
    #[test]
    fn muting_a_space_is_not_muting_the_device() {
        let mut ps = Policies::default();
        ps.set_space(
            "work",
            Policy {
                muted: true,
                quiet: None,
            },
        );
        assert_eq!(
            ps.verdict(Some("work"), Priority::High, 720, 0),
            Some(Status::Muted)
        );
        assert_eq!(ps.verdict(Some("home"), Priority::High, 720, 0), None);
        assert_eq!(ps.verdict(None, Priority::High, 720, 0), None);
    }

    /// The device's reason wins, so "muted" is never reported as "quiet hours".
    #[test]
    fn the_device_reason_is_the_one_reported() {
        let mut ps = Policies::default();
        ps.device.muted = true;
        ps.set_space(
            "work",
            Policy {
                muted: false,
                quiet: Some(quiet("00:00", "23:59", false)),
            },
        );
        assert_eq!(
            ps.verdict(Some("work"), Priority::Normal, 720, 0),
            Some(Status::Muted)
        );
    }

    /// An override that restricts nothing is not stored, so the interface
    /// never shows a space as configured when nothing about it differs.
    #[test]
    fn an_empty_override_is_dropped_rather_than_kept() {
        let mut ps = Policies::default();
        ps.set_space(
            "work",
            Policy {
                muted: true,
                quiet: None,
            },
        );
        assert!(ps.space("work").is_some());
        ps.set_space("work", Policy::default());
        assert!(ps.space("work").is_none());
        assert!(ps.spaces.is_empty(), "no empty entry should linger");
    }

    /// Forgetting matters because rotating mints a space with a fresh id and
    /// a stale override would apply to it unasked.
    #[test]
    fn forgetting_a_space_removes_its_override() {
        let mut ps = Policies::default();
        ps.set_space(
            "work",
            Policy {
                muted: true,
                quiet: None,
            },
        );
        ps.forget("work");
        assert_eq!(ps.verdict(Some("work"), Priority::Normal, 720, 0), None);
    }

    /// A `policy.json` written before spaces existed must still load, or an
    /// upgrade would silently unmute a device somebody muted on purpose.
    #[test]
    fn a_policy_file_from_before_spaces_still_loads() {
        let old = r#"{"muted":true,"quiet":{"from":1320,"to":420,"high_breaks_through":true}}"#;
        let ps: Policies = serde_json::from_str(old).expect("old policy.json should load");
        assert!(ps.device.muted);
        assert_eq!(ps.device.quiet.unwrap().from, 1320);
        assert!(ps.device.quiet.unwrap().high_breaks_through);
        assert!(ps.spaces.is_empty());
    }

    /// And the new shape round-trips, overrides included.
    #[test]
    fn policies_round_trip_through_json() {
        let mut ps = Policies::default();
        ps.device.quiet = Some(quiet("22:00", "07:00", true));
        ps.set_space(
            "work",
            Policy {
                muted: true,
                quiet: None,
            },
        );
        let text = serde_json::to_string(&ps).unwrap();
        let back: Policies = serde_json::from_str(&text).unwrap();
        assert_eq!(ps, back);
    }

    #[test]
    fn times_round_trip() {
        for s in ["00:00", "07:30", "22:00", "23:59"] {
            assert_eq!(format_time(parse_time(s).unwrap()), s);
        }
        assert_eq!(parse_time("24:00"), None);
        assert_eq!(parse_time("12:60"), None);
        assert_eq!(parse_time("nope"), None);
    }
}
