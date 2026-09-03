//! What this device was asked to say, and what became of it.
//!
//! Recorded whether or not it was spoken. A message that arrived while the
//! device was muted is exactly the one worth keeping: it is the only record
//! that it ever came, and without it muting a device silently discards
//! things people meant you to hear.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use voicecast_proto::{Priority, Status};

/// How many messages to keep.
///
/// Enough to scroll back through a day's worth without becoming a log nobody
/// prunes. The oldest fall off the end.
const KEEP: usize = 200;

/// A ceiling on one entry's text, so a spoken document cannot grow the file
/// without limit. Far above anything anyone reads aloud in practice.
const MAX_TEXT: usize = 100_000;

/// One message this device was asked to speak.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    /// Identifies the message, and addresses it for replay.
    pub msg_id: String,
    /// What was said, or would have been.
    pub text: String,
    /// The device it came from. This device's own name for a local message.
    pub from: String,
    /// Unix seconds when it arrived.
    pub at: u64,
    /// How it ended. `Muted` and `QuietHours` are the interesting ones.
    pub status: Status,
    /// The urgency it was sent with.
    pub priority: Priority,
    /// Which space it came in, when this device belongs to more than one.
    #[serde(default)]
    pub space: Option<String>,
}

impl Entry {
    /// Whether this message was never actually heard.
    ///
    /// The reason history exists: these are the ones someone has to go back
    /// and read.
    pub fn unheard(&self) -> bool {
        matches!(
            self.status,
            Status::Muted | Status::QuietHours | Status::Dropped | Status::Cancelled
        )
    }
}

/// Recent messages, newest last.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct History {
    entries: VecDeque<Entry>,
}

impl History {
    /// Where the history lives.
    pub fn default_path() -> Option<PathBuf> {
        crate::identity::config_dir()
            .ok()
            .map(|d| d.join("history.json"))
    }

    /// Load, treating anything unreadable as empty.
    ///
    /// A corrupt history must not stop a device speaking: it is a
    /// convenience, and losing it is a smaller failure than refusing to work.
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    /// Add a message, dropping the oldest once the limit is reached.
    pub fn record(&mut self, mut entry: Entry) {
        if entry.text.len() > MAX_TEXT {
            entry.text.truncate(MAX_TEXT);
        }
        self.entries.push_back(entry);
        while self.entries.len() > KEEP {
            self.entries.pop_front();
        }
    }

    /// Update how a message ended.
    ///
    /// Separate from recording because most messages are accepted long before
    /// they are spoken — the outcome is not known when the entry is made.
    pub fn set_status(&mut self, msg_id: &str, status: Status) {
        if let Some(entry) = self.entries.iter_mut().rev().find(|e| e.msg_id == msg_id) {
            entry.status = status;
        }
    }

    /// One message, for replaying it.
    pub fn get(&self, msg_id: &str) -> Option<&Entry> {
        self.entries.iter().rev().find(|e| e.msg_id == msg_id)
    }

    /// Recent messages, newest first, at most `limit`.
    pub fn recent(&self, limit: usize) -> Vec<Entry> {
        self.entries.iter().rev().take(limit).cloned().collect()
    }

    /// Forget everything.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Write to disk.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            crate::store::create_dir_private(dir)?;
        }
        let text = serde_json::to_string(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        crate::store::write_private(path, text.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, status: Status) -> Entry {
        Entry {
            msg_id: id.into(),
            text: format!("message {id}"),
            from: "Laptop".into(),
            at: 0,
            status,
            priority: Priority::Normal,
            space: None,
        }
    }

    #[test]
    fn the_oldest_falls_off_once_full() {
        let mut history = History::default();
        for i in 0..KEEP + 10 {
            history.record(entry(&format!("m{i}"), Status::Spoken));
        }
        let recent = history.recent(KEEP + 50);
        assert_eq!(recent.len(), KEEP);
        // Newest first, and the first ten are gone.
        assert_eq!(recent[0].msg_id, format!("m{}", KEEP + 9));
        assert!(history.get("m0").is_none());
        assert!(history.get("m10").is_some());
    }

    #[test]
    fn an_outcome_can_be_filled_in_later() {
        let mut history = History::default();
        history.record(entry("m1", Status::Queued));
        history.set_status("m1", Status::Spoken);
        assert_eq!(
            history.get("m1").map(|e| e.status.clone()),
            Some(Status::Spoken)
        );
        // An id that is not there is not an error; it may have fallen off.
        history.set_status("gone", Status::Spoken);
    }

    #[test]
    fn a_refused_message_is_kept_and_marked_unheard() {
        let mut history = History::default();
        history.record(entry("m1", Status::Muted));
        history.record(entry("m2", Status::Spoken));
        let unheard: Vec<String> = history
            .recent(10)
            .into_iter()
            .filter(Entry::unheard)
            .map(|e| e.msg_id)
            .collect();
        assert_eq!(unheard, vec!["m1".to_string()]);
        // The whole text survives, because reading it back is the point.
        assert_eq!(
            history.get("m1").map(|e| e.text.clone()),
            Some("message m1".into())
        );
    }

    #[test]
    fn an_enormous_message_is_bounded() {
        let mut history = History::default();
        let mut big = entry("m1", Status::Spoken);
        big.text = "x".repeat(MAX_TEXT * 2);
        history.record(big);
        assert_eq!(history.get("m1").map(|e| e.text.len()), Some(MAX_TEXT));
    }
}
