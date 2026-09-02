//! Speech synthesis.
//!
//! **All platform divergence in the project lives here or in the Tauri shell.**
//! `voicecast-proto`, `voicecast-text`, and `voicecast-core` stay portable;
//! anything that cannot be expressed portably gets a trait here rather than a
//! `#[cfg]` in the middle of business logic.

mod silent;

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "android")]
pub use android::{
    AndroidEngine, is_battery_exempt, request_battery_exemption, take_pending_invite,
};

pub use silent::SilentEngine;

// espeak-ng is a Unix binary; there is no such thing to spawn on a phone.
#[cfg(unix)]
mod espeak;
#[cfg(unix)]
pub use espeak::EspeakEngine;

// Piper is a native binary we spawn, so it belongs wherever espeak does.
#[cfg(unix)]
mod piper;
#[cfg(unix)]
pub use piper::PiperEngine;

/// A voice offered by an engine.
#[derive(Debug, Clone)]
pub struct Voice {
    /// Stable identifier used in config.
    pub id: String,
    /// Human-readable name shown in the UI.
    pub name: String,
}

/// Why speech is unavailable or degraded.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// No usable engine — e.g. a voice model not yet downloaded.
    #[error("no speech engine available: {0}")]
    Unavailable(String),
}

/// Whether the active engine is the intended one or a fallback.
///
/// Reported to other devices so a degraded receiver explains itself rather
/// than quietly sounding bad. See `docs/architecture.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// The engine the user chose, working normally.
    Full,
    /// A floor engine standing in, e.g. espeak-ng on Linux.
    Fallback,
}

/// A speech engine.
///
/// Implementations wrap the platform's native synthesiser, a bundled neural
/// engine, or later a cloud API. Engine choice is a receiver-side setting —
/// a direct consequence of only text crossing the wire.
pub trait SpeechEngine: Send + Sync {
    /// Whether this engine can speak right now.
    ///
    /// Checked before a message is accepted so `no_engine` reaches the
    /// *sender*. Without it a device with no engine replies "queued" and then
    /// fails silently in its own log, which is precisely the swallowing the
    /// design forbids.
    fn ready(&self) -> Result<(), EngineError> {
        Ok(())
    }

    /// Speak one chunk.
    fn speak(&self, chunk: &str) -> Result<(), EngineError>;

    /// Voices this engine can offer.
    fn voices(&self) -> Vec<Voice>;

    /// Stop immediately, mid-sentence.
    fn stop(&self);

    /// Whether this is the intended engine or a stand-in.
    fn tier(&self) -> Tier;
}
