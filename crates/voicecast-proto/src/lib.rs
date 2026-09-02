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

/// What the CLI asks the local node to do.
///
/// This is the IPC surface, not the wire protocol — it never leaves the
/// machine. Kept deliberately small: the CLI's job is to hand over text and
/// exit, not to hold opinions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Request {
    /// Speak this text. Targets are resolved by the node.
    Speak {
        /// Text to speak. Already validated by the CLI.
        text: String,
        /// Sender's stated urgency.
        priority: Priority,
        /// Device label to speak on. `None` means this machine.
        #[serde(default)]
        to: Option<String>,
    },
    /// Mint an invite ticket for another device.
    Invite,
    /// Join a space using a ticket from another device.
    Join {
        /// The `voicecast://join/...` string.
        ticket: String,
    },
    /// List devices in the space.
    Devices,
    /// Change this device's local label.
    Rename {
        /// The new label.
        name: String,
    },
    /// Stop playback and clear the queue.
    Stop,
    /// Report node health.
    Status,
}

/// What the node tells the CLI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Response {
    /// Message accepted. The CLI exits successfully here by default —
    /// confirmation of actual playback is opt-in via `--wait`.
    Accepted {
        /// Identifies the message, for `voicecast stop --id`.
        msg_id: String,
    },
    /// Terminal state for a message, sent when the caller asked to wait.
    Finished {
        /// How it ended.
        status: Status,
    },
    /// An invite, ready to hand to another device.
    Invite {
        /// The `voicecast://join/...` string.
        url: String,
        /// Seconds until it stops being accepted.
        expires_in: u64,
    },
    /// Joined a space.
    Joined {
        /// How many devices are now in it.
        members: usize,
    },
    /// The device was renamed.
    Renamed {
        /// Its new label.
        name: String,
    },
    /// Devices in the space.
    Devices {
        /// One row per member.
        devices: Vec<DeviceInfo>,
    },
    /// Node health.
    Status {
        /// This device's public key — its address on the network.
        ///
        /// Defaulted so a newer peer can still read an older one's reply.
        /// CBOR is self-describing, but a *missing required* field is still a
        /// hard error — so every field added after v1 must be defaultable.
        #[serde(default)]
        device_id: String,
        /// Where the private key is kept, so a silent fallback to a file is
        /// visible rather than assumed.
        #[serde(default)]
        key_store: String,
        /// Engine currently in use, e.g. "espeak-ng".
        engine: String,
        /// Whether that engine is the intended one or a stand-in.
        fallback: bool,
        /// Messages waiting to be spoken.
        queued: usize,
    },
    /// The node could not do it.
    Error {
        /// Why.
        message: String,
    },
}
/// One device's membership of a space.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Member {
    /// The device's public key, as a string for stable serialisation.
    pub endpoint_id: String,
    /// Local label. Never an identity — renaming breaks nothing.
    pub name: String,
    /// Who vouched for this device.
    pub invited_by: String,
    /// The inviter's signature over the join record.
    pub signature: Vec<u8>,
    /// Unix seconds. Used to order a rejoin after a revocation.
    pub joined_at: u64,
}

impl Member {
    /// The bytes an inviter signs, and a verifier re-derives.
    ///
    /// Deliberately excludes `name`, so renaming a device does not invalidate
    /// its membership.
    pub fn signed_payload(endpoint_id: &str, invited_by: &str, joined_at: u64) -> Vec<u8> {
        format!("voicecast-join-v1:{endpoint_id}:{invited_by}:{joined_at}").into_bytes()
    }
}

/// Messages exchanged between peer devices.
///
/// Distinct from [`Request`]/[`Response`], which never leave the machine.
/// Everything here crosses the network as CBOR over a QUIC stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PeerMessage {
    /// Opens the long-lived control stream.
    Hello {
        /// Highest protocol version this peer speaks.
        proto_version: u16,
        /// Sender's public key.
        endpoint_id: String,
        /// Sender's local label.
        display_name: String,
    },
    /// A device asking to join a space.
    JoinRequest {
        /// The joiner's public key.
        endpoint_id: String,
        /// What the joiner wants to be called.
        display_name: String,
        /// One-time token from the invite, proving this was intended.
        token: String,
    },
    /// The invite was accepted; here is your record and the roster.
    JoinAccepted {
        /// The joiner's signed membership record.
        member: Member,
        /// Everyone else in the space.
        members: Vec<Member>,
    },
    /// The invite was refused.
    JoinRefused {
        /// Why, in terms the joiner can act on.
        reason: String,
    },
    /// Roster state, exchanged when digests differ.
    RosterSync {
        /// Current members.
        members: Vec<Member>,
        /// Revoked ids and when, so tombstones propagate.
        revoked: Vec<(String, u64)>,
    },
    /// Opens a message stream. Chunks follow, then [`PeerMessage::SpeakEnd`].
    SpeakBegin {
        /// Identifies this message for control commands.
        msg_id: String,
        /// Sender's stated urgency.
        priority: Priority,
    },
    /// One sentence-ish unit of text.
    Chunk {
        /// Position in the message, from zero.
        seq: u32,
        /// The text to speak.
        text: String,
    },
    /// Closes a message stream.
    SpeakEnd,
    /// Terminal or in-progress state, sent back on the same stream.
    Report {
        /// How it went.
        status: Status,
    },
}

/// A device as shown by `voicecast devices`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    /// Local label.
    pub name: String,
    /// Public key.
    pub endpoint_id: String,
    /// Whether this row is the device you are asking.
    pub is_self: bool,
}
