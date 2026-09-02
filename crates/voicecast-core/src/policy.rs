//! What this device will and will not say out loud.
//!
//! Receiver-side, deliberately: the sender states urgency, the device that
//! actually makes noise decides whether noise is acceptable right now. A
//! sender cannot mute or unmute anyone else, which is the whole point — an
//! agent marking everything urgent must not be able to wake the house.

use serde::{Deserialize, Serialize};
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

/// Load this device's policy, falling back to "say everything".
///
/// A missing or unreadable file is not an error: a device that has never been
/// configured should speak, not sit silently for a reason nobody can see.
pub fn load() -> Policy {
    path()
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// Persist the policy.
pub fn save(policy: &Policy) -> Result<(), crate::IdentityError> {
    let p = path()?;
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| crate::IdentityError::Store(e.to_string()))?;
    }
    let text = serde_json::to_string_pretty(policy)
        .map_err(|e| crate::IdentityError::Store(e.to_string()))?;
    std::fs::write(&p, text).map_err(|e| crate::IdentityError::Store(e.to_string()))
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
