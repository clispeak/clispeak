//! Speech synthesis.
//!
//! **All platform divergence in the project lives here or in the Tauri shell.**
//! `voicecast-proto`, `voicecast-text`, and `voicecast-core` stay portable;
//! anything that cannot be expressed portably gets a trait here rather than a
//! `#[cfg]` in the middle of business logic.

// Gated with its only callers. `child` exists to wait on and kill a spawned
// process, and both engines that spawn one are excluded from iOS below — so
// on a phone build this compiled a helper nothing could reach. The compiler
// said so, as two dead-code warnings on an `aarch64-apple-ios` check, which
// is a warning nobody would ever have seen: CI runs clippy on Linux only.
#[cfg(any(all(unix, not(target_os = "ios")), windows))]
mod child;
mod rediscovering;
mod silent;

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "android")]
pub use android::{
    AndroidEngine, is_battery_exempt, request_battery_exemption, take_pending_invite,
};

pub use rediscovering::Rediscovering;
pub use silent::SilentEngine;

// espeak-ng is a Unix binary; there is no such thing to spawn on a phone.
// Not iOS. It is unix, so it was included by omission rather than by
// decision — nobody wrote iOS support, the exclusion was simply phrased as
// "unix, and not Android". espeak spawns a child process, which iOS does not
// permit, so this compiled a speech engine into a phone build that could
// never call it (#126).
#[cfg(all(unix, not(target_os = "ios")))]
mod espeak;
#[cfg(all(unix, not(target_os = "ios")))]
pub use espeak::EspeakEngine;

// Piper is a native binary we spawn, and Windows spawns processes just as
// Unix does — so the same engine serves every desktop, which is what makes a
// message sound the same wherever it lands. Kept as a gate rather than
// dropped so this reads as a deliberate list of platforms, not an oversight.
// The platform synthesiser, which is the only speech iOS has — every other
// unix engine here spawns a process and iOS does not permit that (#126).
#[cfg(target_os = "ios")]
mod ios;
#[cfg(target_os = "ios")]
pub use ios::IosEngine;

// Same reasoning as espeak above: iOS is unix and cannot spawn a process.
#[cfg(any(all(unix, not(target_os = "ios")), windows))]
mod piper;
#[cfg(any(all(unix, not(target_os = "ios")), windows))]
pub use piper::PiperEngine;

/// A voice offered by an engine.
#[derive(Debug, Clone)]
pub struct Voice {
    /// Stable identifier used in config.
    pub id: String,
    /// Human-readable name shown in the UI.
    pub name: String,
}

/// How an engine is currently configured.
#[derive(Debug, Clone)]
pub struct VoiceSettings {
    /// Every voice this engine can offer.
    pub available: Vec<Voice>,
    /// Which one is in use.
    pub current: Option<String>,
    /// Speaking rate, where 1.0 is normal.
    pub rate: f32,
}

/// Why speech is unavailable or degraded.
#[derive(Debug, Clone, thiserror::Error)]
pub enum EngineError {
    /// No usable engine — e.g. a voice model not yet downloaded.
    #[error("no speech engine available: {0}")]
    Unavailable(String),
    /// An engine was present, ran, and failed. Nothing was heard.
    ///
    /// Distinct from [`Unavailable`] because the two need opposite responses:
    /// a missing engine is installed, a failing one is diagnosed. Collapsing
    /// them told someone whose audio device was gone to go and download a
    /// voice model.
    ///
    /// [`Unavailable`]: EngineError::Unavailable
    #[error("{command} failed with {code}{}", match .detail {
        Some(d) => format!(": {d}"),
        None => String::new(),
    })]
    Failed {
        /// The command that failed, as a person would name it.
        command: String,
        /// `exit code 1`, or `a signal`.
        code: String,
        /// A bounded tail of what it wrote to stderr, when it wrote anything.
        detail: Option<String>,
    },
}

impl EngineError {
    /// The explanation without the preamble the `Display` impl adds.
    ///
    /// Use this when handing one engine's failure to [`SilentEngine`], which
    /// reports through the same variant: passing the formatted error would
    /// say "no speech engine available" twice in one sentence, and the reader
    /// is already having a bad enough day.
    pub fn reason(&self) -> std::borrow::Cow<'_, str> {
        match self {
            EngineError::Unavailable(reason) => reason.as_str().into(),
            // Built rather than borrowed, because the useful sentence here is
            // assembled from three fields and none of them is it.
            EngineError::Failed { .. } => self.to_string().into(),
        }
    }
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

    /// The voice currently in use, by id.
    fn current_voice(&self) -> Option<String> {
        self.voices().first().map(|v| v.id.clone())
    }

    /// Choose a voice by id.
    ///
    /// Engines with one voice can ignore this; the default refuses rather
    /// than pretending to have changed something.
    fn set_voice(&self, _id: &str) -> Result<(), EngineError> {
        Err(EngineError::Unavailable(
            "this engine has only one voice".into(),
        ))
    }

    /// Speaking rate as a multiplier, where 1.0 is the engine's normal pace.
    fn rate(&self) -> f32 {
        1.0
    }

    /// Set the speaking rate.
    fn set_rate(&self, _rate: f32) -> Result<(), EngineError> {
        Err(EngineError::Unavailable(
            "this engine's rate cannot be changed".into(),
        ))
    }

    /// Stop immediately, mid-sentence.
    fn stop(&self);

    /// Whether this is the intended engine or a stand-in.
    fn tier(&self) -> Tier;
}
