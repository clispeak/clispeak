//! espeak-ng, the guaranteed floor on Linux.
//!
//! It sounds like 1994, and that is the point: every distro ships it, so a
//! receiver can always say *something*. The design would rather be
//! intelligible and ugly than silent — see the fallback discussion in
//! `docs/architecture.md`.
//!
//! Driven as a subprocess rather than via FFI. The C API buys nothing here:
//! espeak-ng does its own audio output, and one process per utterance is
//! trivially cancellable by killing it.

use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use crate::child::Running;
use crate::{EngineError, SpeechEngine, Tier, Voice};

/// Speaks via the `espeak-ng` binary.
pub struct EspeakEngine {
    /// The utterance currently being spoken, so `stop` can kill it.
    current: Mutex<Option<Arc<Running>>>,
    /// Words per minute.
    rate: u32,
}

impl EspeakEngine {
    /// Create an engine, failing if `espeak-ng` is not on `PATH`.
    ///
    /// Checked once here rather than at speak time, so a missing engine is
    /// reported as `no_engine` before a message is accepted rather than
    /// swallowing it later.
    pub fn new() -> Result<Self, EngineError> {
        Command::new("espeak-ng")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .status()
            .map_err(|e| EngineError::Unavailable(format!("espeak-ng not runnable: {e}")))?;

        Ok(Self {
            current: Mutex::new(None),
            rate: 175,
        })
    }
}

impl SpeechEngine for EspeakEngine {
    fn speak(&self, chunk: &str) -> Result<(), EngineError> {
        // Text goes on stdin, never as an argument: a chunk can contain
        // anything an agent emitted, and stdin has no quoting to get wrong.
        let mut child = Command::new("espeak-ng")
            .arg("-s")
            .arg(self.rate.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| EngineError::Unavailable(format!("could not start espeak-ng: {e}")))?;

        {
            use std::io::Write;
            let mut stdin = child
                .stdin
                .take()
                .ok_or_else(|| EngineError::Unavailable("no stdin on espeak-ng".into()))?;
            stdin
                .write_all(chunk.as_bytes())
                .map_err(|e| EngineError::Unavailable(format!("write to espeak-ng: {e}")))?;
        }

        let running = Running::new("espeak-ng", child);
        *self.current.lock().expect("engine lock") = Some(Arc::clone(&running));

        // Waited on outside the lock. Holding it across the wait is what made
        // `stop` park until the utterance had finished and then kill nothing
        // — issue #58 — and the entry has to stay in `current` while we wait,
        // because that is how `stop` finds it.
        let outcome = running.wait();
        *self.current.lock().expect("engine lock") = None;
        outcome
    }

    fn voices(&self) -> Vec<Voice> {
        vec![Voice {
            id: "en".into(),
            name: "espeak-ng English".into(),
        }]
    }

    fn stop(&self) {
        // Taken out under the lock and killed outside it, so this returns in
        // the time a kill takes rather than the length of the utterance.
        let running = self.current.lock().expect("engine lock").take();
        if let Some(running) = running {
            running.kill();
        }
    }

    fn tier(&self) -> Tier {
        // Always a stand-in. On Linux the intended engine is Piper; this one
        // exists so that a device with no voice model still speaks.
        Tier::Fallback
    }
}
