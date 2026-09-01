//! Wire and IPC message types.
//!
//! Pure data: no I/O, no async, no platform code. Encoded as CBOR, which is
//! self-describing so an old peer can skip fields it has never heard of —
//! see `docs/protocol.md` for why that matters here.

#![doc(html_no_source)]

use serde::{Deserialize, Serialize};

/// Protocol version, negotiated in [`Hello`] down to the lowest common.
pub const PROTO_VERSION: u16 = 1;

/// How urgently a message should be spoken.
///
/// The sender expresses intent; the receiver enforces policy. `High` may
/// interrupt, but never overrides mute or quiet hours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    /// Queued, and dropped if the queue is already deep.
    Low,
    /// Queued and spoken in order.
    Normal,
    /// Interrupts, then resumes the interrupted message at its chunk boundary.
    High,
}

/// Terminal or in-progress state of a message on one receiver.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Accepted, waiting behind other messages.
    Queued,
    /// Currently being spoken.
    Speaking,
    /// Finished speaking.
    Spoken,
    /// Device is muted.
    Muted,
    /// Quiet hours are active on the device.
    QuietHours,
    /// No working speech engine — e.g. a voice model not yet downloaded.
    NoEngine,
    /// Sender is not in this device's roster.
    Rejected,
    /// Cancelled before completion.
    Cancelled,
    /// Discarded without being spoken.
    Dropped,
}

/// Sent once per peer on the long-lived control stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hello {
    /// Highest protocol version this peer speaks.
    pub proto_version: u16,
    /// Human-readable label. A local convenience, never an identity.
    pub display_name: String,
}

/// Opens a message stream. Chunks follow, then [`SpeakEnd`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeakBegin {
    /// Identifies this message for `voicecast stop --id`.
    pub msg_id: String,
    /// Which space this was sent in; selects the receiver's policy.
    pub space_id: String,
    /// Sender's stated urgency.
    pub priority: Priority,
}

/// One sentence-ish unit of text. Also the resume point after an interrupt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    /// Position in the message, from zero.
    pub seq: u32,
    /// The text to speak.
    pub text: String,
}

/// Closes a message stream. No more chunks follow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeakEnd;
