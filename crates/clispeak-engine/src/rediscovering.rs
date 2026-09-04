//! An engine that keeps looking for a real one.
//!
//! Discovery ran once, at startup, and the answer was kept for the life of
//! the process. So the first-run path was: open the app, be told Piper is not
//! installed, run `cargo xtask piper`, and be told Piper is not installed —
//! by a node whose statement had stopped being true several seconds earlier.
//! The README said to install it and did not say to restart, because whoever
//! wrote that line did not know it mattered (#84).
//!
//! This wraps the same [`SilentEngine`] reason and re-runs the probe, at most
//! once every [`RETRY`], until something answers. Once an engine is found it
//! is kept: a working Piper does not need looking for again, and re-probing
//! forever would stat the filesystem on every utterance for no reason.
//!
//! [`SilentEngine`]: crate::SilentEngine

use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use crate::{EngineError, SpeechEngine, Tier, Voice};

/// How often to look again while there is still nothing.
///
/// Two seconds is far below how long it takes a person to install something
/// and try again, and far above the cost of the probe, which is a handful of
/// `stat` calls against paths that are usually absent.
const RETRY: Duration = Duration::from_secs(2);

/// What to call when it is worth looking again.
pub type Probe = Box<dyn Fn() -> Option<Arc<dyn SpeechEngine>> + Send + Sync>;

/// Reports why it cannot speak, and keeps checking whether that is still so.
pub struct Rediscovering {
    probe: Probe,
    /// Why there is nothing yet, in the words the sender should read.
    reason: String,
    found: RwLock<Option<Arc<dyn SpeechEngine>>>,
    /// When it is worth asking again. Separate from `found` so a probe can
    /// run without holding the read lock every caller needs.
    next_try: Mutex<Instant>,
}

impl Rediscovering {
    /// Wrap a probe, explaining meanwhile why this device cannot speak.
    pub fn new(reason: impl Into<String>, probe: Probe) -> Self {
        Self {
            probe,
            reason: reason.into(),
            found: RwLock::new(None),
            next_try: Mutex::new(Instant::now()),
        }
    }

    /// The real engine, if there is one yet.
    fn engine(&self) -> Option<Arc<dyn SpeechEngine>> {
        if let Some(found) = self.found.read().expect("engine lock").clone() {
            return Some(found);
        }
        // One probe at a time, and not more often than `RETRY`. Without the
        // guard a burst of messages against a machine with no engine would
        // each run their own filesystem walk.
        {
            let mut next = self.next_try.lock().expect("probe lock");
            if Instant::now() < *next {
                return None;
            }
            *next = Instant::now() + RETRY;
        }
        let found = (self.probe)()?;
        *self.found.write().expect("engine lock") = Some(Arc::clone(&found));
        Some(found)
    }

    fn nothing_yet(&self) -> EngineError {
        EngineError::Unavailable(self.reason.clone())
    }
}

impl SpeechEngine for Rediscovering {
    fn ready(&self) -> Result<(), EngineError> {
        match self.engine() {
            Some(e) => e.ready(),
            None => Err(self.nothing_yet()),
        }
    }

    fn speak(&self, chunk: &str) -> Result<(), EngineError> {
        match self.engine() {
            Some(e) => e.speak(chunk),
            None => Err(self.nothing_yet()),
        }
    }

    fn voices(&self) -> Vec<Voice> {
        self.engine().map(|e| e.voices()).unwrap_or_default()
    }

    fn stop(&self) {
        // Only what has actually been handed something to say. Probing here
        // would mean `stop` could start an engine, which is absurd.
        if let Some(e) = self.found.read().expect("engine lock").clone() {
            e.stop();
        }
    }

    fn set_voice(&self, id: &str) -> Result<(), EngineError> {
        match self.engine() {
            Some(e) => e.set_voice(id),
            None => Err(self.nothing_yet()),
        }
    }

    fn current_voice(&self) -> Option<String> {
        self.engine().and_then(|e| e.current_voice())
    }

    fn set_rate(&self, rate: f32) -> Result<(), EngineError> {
        match self.engine() {
            Some(e) => e.set_rate(rate),
            None => Err(self.nothing_yet()),
        }
    }

    fn tier(&self) -> Tier {
        self.engine().map_or(Tier::Fallback, |e| e.tier())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Works;
    impl SpeechEngine for Works {
        fn speak(&self, _: &str) -> Result<(), EngineError> {
            Ok(())
        }
        fn voices(&self) -> Vec<Voice> {
            Vec::new()
        }
        fn stop(&self) {}
        fn tier(&self) -> Tier {
            Tier::Full
        }
    }

    #[test]
    fn an_engine_installed_after_the_node_started_is_found() {
        // The whole point: this used to say "not installed" until restart,
        // about a machine where it had been installed a minute ago (#84).
        let ready = Arc::new(AtomicUsize::new(0));
        let flag = Arc::clone(&ready);
        let engine = Rediscovering::new(
            "piper is not installed",
            Box::new(move || {
                (flag.load(Ordering::SeqCst) == 1).then(|| Arc::new(Works) as Arc<dyn SpeechEngine>)
            }),
        );

        assert!(engine.ready().is_err(), "nothing there yet");
        ready.store(1, Ordering::SeqCst);
        std::thread::sleep(RETRY + Duration::from_millis(50));
        assert!(engine.ready().is_ok(), "found once it exists");
        assert_eq!(engine.tier(), Tier::Full);
    }

    #[test]
    fn the_probe_is_not_run_for_every_message() {
        // A machine with no engine at all must not walk the filesystem once
        // per utterance.
        let runs = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&runs);
        let engine = Rediscovering::new(
            "nothing here",
            Box::new(move || {
                counter.fetch_add(1, Ordering::SeqCst);
                None
            }),
        );
        for _ in 0..50 {
            let _ = engine.speak("hello");
        }
        assert_eq!(runs.load(Ordering::SeqCst), 1, "one probe, not fifty");
    }

    #[test]
    fn the_reason_is_the_one_a_sender_reads() {
        let engine = Rediscovering::new("piper is not installed", Box::new(|| None));
        let why = engine.speak("hello").unwrap_err().to_string();
        assert!(why.contains("piper is not installed"), "{why}");
    }
}
