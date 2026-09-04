//! What this device was asked to say, and what became of it.
//!
//! Recorded whether or not it was spoken. A message that arrived while the
//! device was muted is exactly the one worth keeping: it is the only record
//! that it ever came, and without it muting a device silently discards
//! things people meant you to hear.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use clispeak_proto::{Priority, Status};
use serde::{Deserialize, Serialize};

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

    /// Write to disk, here and now.
    ///
    /// The blocking form, kept for tests and for the one place a caller
    /// genuinely has to know the bytes have landed. Everything on the path of
    /// a message being spoken goes through [`Saver`] instead — see there.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        crate::store::write_private(path, &self.to_bytes()?)
    }

    /// The file's contents, without writing them.
    ///
    /// Split out so a caller can serialise while it holds the lock and hand
    /// the bytes to [`Saver`] without holding it across a disk write.
    pub fn to_bytes(&self) -> std::io::Result<Vec<u8>> {
        serde_json::to_vec(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }
}

/// Writes the history to disk, off whatever thread asked.
///
/// **Every `SpeakBegin` and every outcome used to write the whole file**,
/// under a `std::sync::Mutex`, on a tokio worker — up to 200 entries of
/// 100,000 characters, and `write_private` calls `sync_all` before it
/// renames. So delivering a message waited on an fsync, on a phone, with
/// latency proportional to how much history the device had accumulated
/// (#78).
///
/// This is a thread and a one-slot mailbox. The slot holds the *latest*
/// snapshot rather than a queue of them, which is what makes a burst of
/// outcomes cost one write instead of five: an older snapshot that has not
/// been written yet is not a lost update, it is a state the newer one
/// already contains.
///
/// A thread rather than `spawn_blocking` because the outcome callback is
/// handed to the queue and can run without a tokio runtime around it, and a
/// history write that panics because there is no reactor would be a worse
/// bug than the one being fixed.
pub struct Saver {
    slot: std::sync::Arc<Slot>,
    /// Kept so a `put` can still write when the thread could not be started.
    path: PathBuf,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// The mailbox, shared with the writer thread.
struct Slot {
    state: std::sync::Mutex<SlotState>,
    /// Woken when something is put in, and when a write finishes.
    bell: std::sync::Condvar,
}

#[derive(Default)]
struct SlotState {
    pending: Option<Vec<u8>>,
    stopping: bool,
    /// Counts snapshots handed over.
    queued: u64,
    /// The highest `queued` whose snapshot has reached the disk.
    ///
    /// Not a count of writes: coalescing means one write can satisfy several
    /// `put`s, and the snapshot taken carries whatever every earlier one
    /// contained. Counting writes instead would leave `flush` waiting for
    /// writes that correctly never happened.
    written: u64,
}

impl Saver {
    /// Start writing to `path`.
    pub fn spawn(path: PathBuf) -> Self {
        let slot = std::sync::Arc::new(Slot {
            state: std::sync::Mutex::new(SlotState::default()),
            bell: std::sync::Condvar::new(),
        });
        let mine = std::sync::Arc::clone(&slot);
        let mine_path = path.clone();
        let thread = std::thread::Builder::new()
            .name("clispeak-history".into())
            .spawn(move || write_loop(&mine, &mine_path))
            .ok();
        Self { slot, path, thread }
    }

    /// Hand over the latest snapshot, replacing any not yet written.
    pub fn put(&self, bytes: Vec<u8>) {
        // No thread means the write has to happen here or not at all, and
        // "not at all" is a history that silently stops recording. Slow is
        // the failure this whole type exists to avoid; losing the file is a
        // worse one.
        if self.thread.is_none() {
            if let Err(e) = crate::store::write_private(&self.path, &bytes) {
                eprintln!("could not save history: {e}");
            }
            return;
        }
        let mut state = self.slot.state.lock().expect("history slot");
        state.pending = Some(bytes);
        state.queued += 1;
        self.slot.bell.notify_all();
    }

    /// Wait until everything handed over so far has been written.
    ///
    /// For shutdown, and for a test that needs the file to exist. Ordinary
    /// recording never calls this — waiting is the thing being removed.
    pub fn flush(&self) {
        let mut state = self.slot.state.lock().expect("history slot");
        let want = state.queued;
        while state.written < want && self.thread.is_some() {
            state = self.slot.bell.wait(state).expect("history slot");
        }
    }
}

impl Drop for Saver {
    fn drop(&mut self) {
        {
            let mut state = self.slot.state.lock().expect("history slot");
            state.stopping = true;
            self.slot.bell.notify_all();
        }
        // Joined rather than detached: the last snapshot has to reach the
        // disk, and a process exiting is not a reason to lose it.
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn write_loop(slot: &Slot, path: &Path) {
    loop {
        let (bytes, mark) = {
            let mut state = slot.state.lock().expect("history slot");
            while state.pending.is_none() && !state.stopping {
                state = slot.bell.wait(state).expect("history slot");
            }
            let mark = state.queued;
            match state.pending.take() {
                Some(bytes) => (bytes, mark),
                // Nothing left and asked to stop. Anything put in after this
                // point cannot arrive: `Drop` sets `stopping` and joins.
                None => return,
            }
        };
        if let Err(e) = crate::store::write_private(path, &bytes) {
            eprintln!("could not save history: {e}");
        }
        let mut state = slot.state.lock().expect("history slot");
        // Marked done even when the write failed. This says the writer got
        // to it, which is what a flush is waiting for; the failure has
        // already been reported and waiting longer will not mend it.
        state.written = state.written.max(mark);
        slot.bell.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn entry(id: &str, status: Status) -> Entry {
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

#[cfg(test)]
mod saver_tests {
    use super::tests::entry;
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("clispeak-history-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch directory");
        dir.join(name)
    }

    #[test]
    fn a_burst_of_snapshots_lands_as_the_last_one() {
        // The property that makes this worth a thread: five outcomes in a
        // row cost one write, not five, and the file ends up holding the
        // newest state rather than whichever write finished last (#78).
        let path = scratch("burst.json");
        let _ = std::fs::remove_file(&path);

        let saver = Saver::spawn(path.clone());
        let mut history = History::default();
        for i in 0..5 {
            history.record(entry(&format!("m{i}"), Status::Spoken));
            saver.put(history.to_bytes().expect("serialise"));
        }
        saver.flush();

        let written = History::load(&path);
        assert_eq!(written.recent(10).len(), 5, "the newest snapshot, whole");
        assert_eq!(written.recent(1)[0].msg_id, "m4");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn dropping_the_saver_writes_what_was_still_pending() {
        // A node closing must not lose the last outcome it recorded.
        let path = scratch("pending.json");
        let _ = std::fs::remove_file(&path);

        let mut history = History::default();
        history.record(entry("only", Status::Muted));
        {
            let saver = Saver::spawn(path.clone());
            saver.put(history.to_bytes().expect("serialise"));
        }

        let written = History::load(&path);
        assert_eq!(written.recent(1).len(), 1);
        assert_eq!(written.recent(1)[0].msg_id, "only");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_flush_with_nothing_pending_returns() {
        // `close` flushes unconditionally, including on a node that never
        // recorded anything.
        let saver = Saver::spawn(scratch("never.json"));
        saver.flush();
        saver.flush();
    }
}
