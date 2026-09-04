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
    /// The device could not be reached at all.
    Unreachable,
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
    /// Identifies this message for `clispeak stop --id`.
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
        /// Wait for every target to finish before replying.
        ///
        /// Off by default: an agent firing notifications should not block on
        /// playback. On, the caller learns what actually happened.
        #[serde(default)]
        wait: bool,
        /// A voice to use for this message, if the receiver has it.
        ///
        /// A request, not an instruction: engines differ per device, so a
        /// receiver that has never heard of the voice speaks in its own
        /// rather than refusing.
        #[serde(default)]
        voice: Option<String>,
        /// How long to wait for a terminal state, in seconds.
        #[serde(default)]
        timeout_secs: Option<u64>,
    },
    /// Report which devices a selector names, without speaking anything.
    Resolve {
        /// The selector to resolve.
        #[serde(default)]
        to: Option<String>,
    },
    /// Mint an invite ticket for another device.
    Invite {
        /// Which space to invite into, by its local name. `None` is the
        /// default space.
        #[serde(default)]
        space: Option<String>,
    },
    /// Join a space using a ticket from another device.
    Join {
        /// The `clispeak://join/...` string.
        ticket: String,
        /// What to call the joined space on this device.
        ///
        /// `None` takes the name the inviter uses. A label is local — it is
        /// how this device addresses the space in `work/laptop` — so two
        /// people can reasonably disagree about it, and the one doing the
        /// joining gets the last word.
        #[serde(default)]
        label: Option<String>,
    },
    /// Read an invite without acting on it.
    ///
    /// Purely local: it parses the ticket and reports what joining would do.
    /// No device is contacted and the token is not spent, so this can be run
    /// as the code is pasted. It exists because the destination is written
    /// inside the ticket by whoever minted it — the joining device cannot
    /// choose it, so the only honest thing to offer is to read it out first.
    Preview {
        /// The `clispeak://join/...` string.
        ticket: String,
    },
    /// List devices in the space.
    Devices,
    /// Bring the app's window back from the tray.
    Show,
    /// Shut the node down.
    Quit,
    /// Remove another device from this space.
    Revoke {
        /// The device's label.
        name: String,
        /// Which space to remove it from. `None` is the default space.
        #[serde(default)]
        space: Option<String>,
    },
    /// Leave a space, keeping this device's identity.
    Leave {
        /// Which space. `None` is the default space.
        #[serde(default)]
        space: Option<String>,
    },
    /// Replace a space with a fresh one, locking every other device out.
    Rotate {
        /// Which space to replace. `None` is the default space.
        #[serde(default)]
        space: Option<String>,
    },
    /// List the spaces this device belongs to.
    Spaces,
    /// Found a new space from this device and make it the default.
    NewSpace {
        /// What to call it locally.
        label: String,
    },
    /// Drop one space, keeping the others.
    LeaveSpace {
        /// The space's local name.
        label: String,
    },
    /// Choose which space bare device names resolve in.
    DefaultSpace {
        /// The space's local name.
        label: String,
    },
    /// Rename a space locally.
    RenameSpace {
        /// Its current local name.
        label: String,
        /// What to call it instead.
        to: String,
    },
    /// Change this device's local label.
    Rename {
        /// The new label.
        name: String,
    },
    /// Stop playback, on this device or on named ones.
    Stop {
        /// Which devices. `None` means this machine.
        #[serde(default)]
        to: Option<String>,
        /// One message by id, rather than everything.
        #[serde(default)]
        msg_id: Option<String>,
    },
    /// Report node health.
    Status,
    /// Abandon the current message and carry on with the queue.
    Skip {
        /// Which devices. `None` means this machine.
        #[serde(default)]
        to: Option<String>,
    },
    /// Hold speech without discarding it.
    Pause {
        /// Which devices. `None` means this machine.
        #[serde(default)]
        to: Option<String>,
    },
    /// Start speaking again after a pause.
    Resume {
        /// Which devices. `None` means this machine.
        #[serde(default)]
        to: Option<String>,
    },
    /// Report what is playing and what is waiting.
    Queue,
    /// Recent messages this device was asked to speak.
    History {
        /// How many to return, newest first.
        #[serde(default)]
        limit: Option<usize>,
    },
    /// Speak a message from the history again, here.
    Replay {
        /// Which message.
        msg_id: String,
    },
    /// Forget the history.
    ClearHistory,
    /// Report this device's speaking policy.
    Policy,
    /// Silence this device, or let it speak again.
    SetMute {
        /// Whether to be silent.
        muted: bool,
        /// Which space to mute, or `None` for the whole device.
        ///
        /// Note that `None` means *the device* here, not the default space —
        /// unlike [`Request::Revoke`] and its neighbours, where an absent
        /// space means "the default one". The asymmetry is deliberate: the
        /// device-wide switch is the one most people ever touch, so it is the
        /// one that needs no argument.
        #[serde(default)]
        space: Option<String>,
    },
    /// Set or clear the daily quiet window.
    SetQuiet {
        /// Start as `HH:MM` local time. `None` in either end clears the
        /// window entirely, which is how quiet hours are turned off.
        from: Option<String>,
        /// End as `HH:MM` local time. May be earlier than `from`.
        to: Option<String>,
        /// Whether `high` may break through. Off unless asked for.
        #[serde(default)]
        high_breaks_through: bool,
        /// Which space the window belongs to, or `None` for the whole device.
        ///
        /// Same reading as [`Request::SetMute::space`].
        #[serde(default)]
        space: Option<String>,
    },
}

/// What the node tells the CLI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Response {
    /// Message accepted. The CLI exits successfully here by default —
    /// confirmation of actual playback is opt-in via `--wait`.
    Accepted {
        /// Identifies the message, for `clispeak stop --id`.
        msg_id: String,
    },
    /// Terminal state for a message, sent when the caller asked to wait.
    Finished {
        /// How it ended.
        status: Status,
    },
    /// An invite, ready to hand to another device.
    Invite {
        /// The `clispeak://join/...` string.
        url: String,
        /// Seconds until it stops being accepted.
        expires_in: u64,
    },
    /// Joined a space.
    Joined {
        /// How many devices are now in it.
        members: usize,
        /// What this device now calls it locally.
        ///
        /// Worth returning because joining a second space has to name it
        /// something, and the person who joined should be told what rather
        /// than having to go and look.
        #[serde(default)]
        space: String,
    },
    /// What an invite would do, without doing it.
    Preview {
        /// The inviter's name for the space, when the ticket carries one.
        ///
        /// Absent for a ticket minted before labels travelled, which is not
        /// an error — it means "the inviting device's default space", and the
        /// interface should say that rather than invent a name.
        #[serde(default)]
        label: Option<String>,
        /// Seconds until the invite stops being accepted.
        expires_in: u64,
        /// The inviting device's public key, for comparing against its screen.
        endpoint_id: String,
    },
    /// The request was carried out but has nothing to report.
    Done,
    /// Per-device outcome of a message.
    Report {
        /// Identifies the message.
        msg_id: String,
        /// One entry per target.
        targets: Vec<TargetResult>,
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
        /// Whether this device is silenced.
        #[serde(default)]
        muted: bool,
        /// The quiet window as `HH:MM-HH:MM`, if one is set.
        ///
        /// Pre-formatted rather than structured: `status` is read by a
        /// person, and the machine-readable shape is [`Response::Policy`].
        #[serde(default)]
        quiet: Option<String>,
        /// Why the engine cannot speak, when it cannot.
        ///
        /// The node has always known this — it reaches anyone who *sends* a
        /// message — but the status line, which is where somebody looks
        /// first, had to guess from an empty voice list and guessed
        /// "starting…" at an engine that was never going to start.
        #[serde(default)]
        engine_reason: Option<String>,
    },
    /// Recent messages this device was asked to speak.
    History {
        /// Newest first.
        entries: Vec<HistoryEntry>,
    },
    /// What a control command did on each device.
    Controlled {
        /// One entry per target.
        targets: Vec<TargetResult>,
    },
    /// The spaces this device belongs to.
    Spaces {
        /// One row per space.
        spaces: Vec<SpaceRow>,
    },
    /// A space was left.
    Left {
        /// Its local name.
        space: String,
        /// Devices told at the time. The rest find out when next reached.
        unreached: usize,
        /// Whether the space is gone, or was replaced by a fresh empty one
        /// because it was the only one this device had.
        refounded: bool,
    },
    /// The space was replaced.
    Rotated {
        /// Which space, by its local name.
        ///
        /// Carried because the reply could not otherwise tell "replaced work"
        /// from "replaced home" — which is exactly what let a bug replacing
        /// the wrong space go unnoticed.
        #[serde(default)]
        space: String,
        /// Devices that were in the old space, so they can be re-invited.
        devices: Vec<String>,
    },
    /// The devices a selector names.
    Targets {
        /// Their labels, in the order a message would reach them.
        devices: Vec<String>,
    },
    /// What this device is speaking and what is waiting.
    Queue {
        /// The message being spoken, if any.
        #[serde(default)]
        speaking: Option<String>,
        /// Messages waiting, in the order they will be spoken.
        #[serde(default)]
        pending: Vec<String>,
        /// Whether speech is held.
        #[serde(default)]
        paused: bool,
    },
    /// This device's speaking policy.
    Policy {
        /// Silenced indefinitely.
        muted: bool,
        /// Quiet window start, `HH:MM` local, if one is set.
        #[serde(default)]
        quiet_from: Option<String>,
        /// Quiet window end, `HH:MM` local, if one is set.
        #[serde(default)]
        quiet_to: Option<String>,
        /// Whether `high` may break through quiet hours.
        #[serde(default)]
        high_breaks_through: bool,
        /// Per-space restrictions on top of the device policy.
        ///
        /// Only spaces that actually restrict something appear. Defaulted so
        /// a reader built before this existed sees the device policy and
        /// nothing else, rather than failing to parse.
        #[serde(default)]
        spaces: Vec<SpacePolicy>,
    },
    /// The node could not do it.
    Error {
        /// Why, for a person or an agent to read.
        message: String,
        /// What sort of failure this is, when the caller has to branch.
        ///
        /// A string rather than an enum on purpose. An unknown enum variant
        /// fails the whole decode, so a newer node teaching this field a new
        /// value would break every older CLI — the same trap that makes
        /// adding a `Response` variant unsafe. An unrecognised string simply
        /// matches nothing, which is exactly the behaviour a reader wants
        /// from a kind it has never heard of. Compare against the
        /// [`error_kind`] constants. Defaulted, so an older node sends none.
        #[serde(default)]
        kind: Option<String>,
    },
}

/// Values [`Response::Error`]'s `kind` can take.
///
/// Constants rather than an enum — see the field's own note. Each exists
/// because a caller does something different about it, which is the only
/// reason to distinguish a failure at all.
pub mod error_kind {
    /// The selector named no device this node could reach.
    ///
    /// Distinct from a malformed command because the command was well formed:
    /// `docs/cli.md` has promised exit code 2 for it since the table was
    /// written, and the CLI returned 1 for everything (#66).
    pub const NO_TARGET: &str = "no-target";
}

impl Response {
    /// A failure with nothing more to say about it than why.
    pub fn error(message: impl Into<String>) -> Self {
        Self::Error {
            message: message.into(),
            kind: None,
        }
    }

    /// A failure because the selector matched no device.
    pub fn no_target(message: impl Into<String>) -> Self {
        Self::Error {
            message: message.into(),
            kind: Some(error_kind::NO_TARGET.to_string()),
        }
    }
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
    /// Unix seconds when the label last changed.
    ///
    /// Names sit outside the signature so they *can* change, which means a
    /// merge cannot tell a renamed entry from a stale one by `joined_at` —
    /// renaming does not alter it. This does, so the newer label wins.
    /// Defaulted: entries written before this field existed sort oldest,
    /// which is correct.
    #[serde(default)]
    pub renamed_at: u64,
}

impl Member {
    /// The bytes an inviter signs, and a verifier re-derives.
    ///
    /// Deliberately excludes `name`, so renaming a device does not invalidate
    /// its membership.
    ///
    /// # These bytes are not a name
    ///
    /// `clispeak-join-v1` is a **domain separator**: it is data that every
    /// signature in existence was computed over, on every device, including
    /// ones that will never be rebuilt. Changing it does not rename anything.
    /// It makes every membership record on every paired device fail to
    /// verify, at once, with no error and no message — the roster simply
    /// empties and both ends report a successful join over nothing.
    ///
    /// That is not hypothetical. A blanket rename of the project changed this
    /// line from `voicecast-join-v1`, and it cost a day of diagnosis and
    /// every pairing on three devices (decision 83).
    ///
    /// A test below pins the exact bytes, because nothing else can: signing
    /// and verifying use this same function, so **a test that signs and
    /// verifies agrees with itself no matter what this says**. Only a written
    /// down expected value disagrees.
    pub fn signed_payload(endpoint_id: &str, invited_by: &str, joined_at: u64) -> Vec<u8> {
        format!("clispeak-join-v1:{endpoint_id}:{invited_by}:{joined_at}").into_bytes()
    }
}

#[cfg(test)]
mod signed_payload_tests {
    use super::Member;

    /// The bytes, written down.
    ///
    /// **If this test fails, do not update the expected value.** It is not
    /// describing the code; it is describing the signatures already sitting
    /// on every paired device, which cannot be updated. A change here is a
    /// break of every existing pairing, and the only honest ways through it
    /// are to put the constant back, or to accept the break deliberately and
    /// tell everyone to re-pair — which is what decision 83 records having
    /// done, once, and why this test exists so it is never done by accident.
    ///
    /// The value is spelled out rather than built with `format!`, because a
    /// test that derives its expectation the same way the code does is a test
    /// that cannot disagree with it.
    #[test]
    fn the_signed_payload_is_a_fixed_shape_that_past_signatures_depend_on() {
        // Compared as text, not bytes: a failure here has to be readable at
        // a glance, and two 38-element `u8` arrays are not.
        let payload = String::from_utf8(Member::signed_payload("aaaa", "bbbb", 1_700_000_000))
            .expect("the payload is ascii");
        assert_eq!(
            payload, "clispeak-join-v1:aaaa:bbbb:1700000000",
            "the signed payload changed shape. Every membership record on \
             every paired device was signed over the old one and none of them \
             will verify against this. Read `Member::signed_payload`'s doc \
             comment before touching this expectation"
        );
    }

    /// The other half: what is *not* in it.
    ///
    /// A name outside the payload is what lets a device be renamed without
    /// losing its membership, and `renamed_at` sits outside for the same
    /// reason. Adding either would break every signature exactly as changing
    /// the separator does, and would do it while looking like a feature.
    #[test]
    fn a_name_is_not_part_of_what_is_signed() {
        let payload = Member::signed_payload("aaaa", "bbbb", 1);
        let text = String::from_utf8(payload).expect("ascii");
        assert!(!text.contains("Phone"), "nothing about a label is in here");
        assert_eq!(text.split(':').count(), 4, "separator, two ids, a time");
    }
}

/// A control command, aimed at whichever device is speaking.
///
/// Separate from [`Request`] because these travel between devices: stopping a
/// phone from a laptop is the whole point, and a "stop" that only ever meant
/// "stop here" would leave a device talking with no way to quiet it but
/// walking over to it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Control {
    /// Stop speaking and clear the queue, or drop one message by id.
    Stop {
        /// One message, rather than everything.
        #[serde(default)]
        msg_id: Option<String>,
    },
    /// Abandon the current message and carry on with the queue.
    Skip,
    /// Hold speech without discarding it.
    Pause,
    /// Start speaking again after a pause.
    Resume,
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
        /// Which space was joined, so both sides agree on its id.
        #[serde(default)]
        space: Option<String>,
        /// What the inviter calls it.
        ///
        /// Sent as well as riding on the ticket because the two can disagree:
        /// a space renamed between minting an invite and it being scanned is
        /// stale on the ticket and current here. Defaulted, so a peer that
        /// predates this simply sends nothing and the joiner falls back.
        #[serde(default)]
        label: Option<String>,
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
        /// Which space this roster is for.
        ///
        /// Defaulted so a peer that predates several spaces still syncs: with
        /// no id the receiver works out which of its spaces the sender is in,
        /// which is unambiguous whenever they share only one.
        #[serde(default)]
        space: Option<String>,
    },
    /// Opens a message stream. Chunks follow, then [`PeerMessage::SpeakEnd`].
    SpeakBegin {
        /// Identifies this message for control commands.
        msg_id: String,
        /// Sender's stated urgency.
        priority: Priority,
        /// Whether the sender wants to be told when speaking finishes, rather
        /// than merely that the message was accepted.
        #[serde(default)]
        wait: bool,
        /// A voice the sender would like, if this device has it.
        #[serde(default)]
        voice: Option<String>,
        /// Which space this was sent in, selecting the receiver's policy.
        ///
        /// Defaulted for the same reason as on [`PeerMessage::RosterSync`].
        #[serde(default)]
        space: Option<String>,
        /// How long the sender is prepared to wait, in seconds.
        ///
        /// Only ever set when the caller passed `--timeout`. Absent means the
        /// receiver decides, which it is far better placed to do: the length
        /// of the text is the smaller half of the answer, and the engine, its
        /// rate and the queue in front of it all live on that device.
        ///
        /// Defaulted so an older peer, which sends nothing here, is read as
        /// having expressed no preference rather than as asking for zero.
        #[serde(default)]
        timeout_secs: Option<u64>,
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
    /// Do something to what this device is saying.
    Control {
        /// What to do.
        control: Control,
    },
    /// Terminal or in-progress state, sent back on the same stream.
    Report {
        /// How it went.
        status: Status,
        /// Why, when the status alone does not say.
        ///
        /// `Rejected` used to mean both "you are not in this space" and
        /// "that message was refused", which an agent has to tell apart to
        /// know whether to fix the pairing or shorten the text. Defaulted, so
        /// a peer running an older build simply sends none.
        #[serde(default)]
        detail: Option<String>,
    },
}

/// What happened on one device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetResult {
    /// The device's label.
    pub device: String,
    /// Which device that label actually resolved to.
    ///
    /// A label is local, freely chosen, and not unique: two devices can carry
    /// the same one, and this device's own name shadows a peer that shares it.
    /// When that happened the report said "spoken" and named a label that was
    /// true of several devices, so the one thing it exists to say — which
    /// device — was the thing it could not. Defaulted, so a node built before
    /// this still parses.
    #[serde(default)]
    pub endpoint_id: String,
    /// How it ended.
    pub status: Status,
    /// How long speaking took, when the caller waited for it.
    #[serde(default)]
    pub took_ms: Option<u64>,
    /// Why, when the status alone does not explain it.
    #[serde(default)]
    pub detail: Option<String>,
}

/// One message this device was asked to speak, spoken or not.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// Identifies the message, and addresses it for replay.
    pub msg_id: String,
    /// What was said, or would have been.
    pub text: String,
    /// The device it came from.
    pub from: String,
    /// Unix seconds when it arrived.
    pub at: u64,
    /// How it ended.
    pub status: Status,
    /// The urgency it was sent with.
    pub priority: Priority,
    /// Whether it was never actually heard, and so is worth going back to.
    #[serde(default)]
    pub unheard: bool,
}

/// One space, as shown by `clispeak space list`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpaceRow {
    /// This device's own name for it. Local, like a device label.
    pub label: String,
    /// How many devices are in it.
    pub devices: usize,
    /// Whether bare device names resolve here.
    pub is_default: bool,
    /// Whether this device founded it.
    pub founded_here: bool,
}

/// One space's extra restrictions, on top of the device policy.
///
/// Carries the *label* rather than the space id, because this is what a person
/// reads. The id never appears in the interface, and a control that acts on a
/// space has to be able to say which one out loud.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpacePolicy {
    /// This device's own name for the space.
    pub label: String,
    /// Silenced indefinitely, for this space alone.
    #[serde(default)]
    pub muted: bool,
    /// Quiet window start, `HH:MM` local, if this space sets one.
    #[serde(default)]
    pub quiet_from: Option<String>,
    /// Quiet window end, `HH:MM` local, if this space sets one.
    #[serde(default)]
    pub quiet_to: Option<String>,
    /// Whether `high` may break through *this space's* window.
    #[serde(default)]
    pub high_breaks_through: bool,
}

/// A device as shown by `clispeak devices`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    /// Local label.
    pub name: String,
    /// Public key.
    pub endpoint_id: String,
    /// Whether this row is the device you are asking.
    pub is_self: bool,
    /// Seconds since this device was last reached, if ever.
    ///
    /// `None` means never seen — a device that joined but has not been in
    /// contact since. Absence of news is genuinely different from bad news.
    #[serde(default)]
    pub last_seen_secs: Option<u64>,
    /// Which space it is in, when this device belongs to more than one.
    ///
    /// `None` when there is only one space, so the common case shows no
    /// column that would always read the same.
    #[serde(default)]
    pub space: Option<String>,
}
