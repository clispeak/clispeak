//! An engine that cannot speak, and says so.
//!
//! Not a stub to be quietly tolerated: `no_engine` is a designed status, and
//! the architecture is explicit that a device which cannot speak must report
//! that rather than swallow the message. A receiver in this state still joins
//! spaces, still authorises senders, and still tells them plainly why nothing
//! was heard.
//!
//! Used on platforms whose real engine has not been wired up yet, and as the
//! honest floor if a platform engine fails to initialise.

use crate::{EngineError, SpeechEngine, Tier, Voice};

/// Reports `no_engine` for everything it is asked to say.
pub struct SilentEngine {
    reason: String,
}

impl SilentEngine {
    /// Create one, explaining why this device cannot speak.
    ///
    /// The reason reaches the sender, so write it for whoever has to act on
    /// it rather than for a log file.
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl SpeechEngine for SilentEngine {
    fn ready(&self) -> Result<(), EngineError> {
        Err(EngineError::Unavailable(self.reason.clone()))
    }

    fn speak(&self, _chunk: &str) -> Result<(), EngineError> {
        Err(EngineError::Unavailable(self.reason.clone()))
    }

    fn voices(&self) -> Vec<Voice> {
        Vec::new()
    }

    fn stop(&self) {}

    fn tier(&self) -> Tier {
        Tier::Fallback
    }
}
