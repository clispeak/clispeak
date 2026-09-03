# Architecture

> Status: draft — design in progress. Sections marked **OPEN** are undecided.

## What this is

A command-line tool that lets an agent speak text aloud on one or more of your
devices:

```
$ voicecast "build finished"                 # speak on default target(s)
$ voicecast --to phone "needs your input"    # speak on a named device
$ cat CHANGELOG.md | voicecast --to desk     # long-form, piped
```

Devices reach each other peer-to-peer. There is no server to run.

## Goals

- **Trivial to set up.** Install, scan a QR code, done.
- **Works anywhere.** Not just on the same wifi — phones on cellular too.
- **No hosted infrastructure.** Nothing the user has to deploy or pay for.
- **Any length.** The CLI has no opinion about how much text you give it.
- **Consistent across platforms.** Same app, same UI, five targets.

## Non-goals (for now)

- Smart speakers as direct receivers (see *Deferred* below).
- Cloud/API TTS voices in v1 (the engine interface anticipates them).
- Voice consistency across devices — each device speaks in its own voice.
- Sharing a space with another person. A space is one user's own devices.

## Core invariant

> **Only text crosses the wire. The receiver always renders it.**

No audio is ever transmitted. This keeps payloads tiny (~50KB of text is
five minutes of speech), removes the audio codec and streaming machinery
entirely, and makes each device's voice a purely local concern.

## Components

### `voicecast` — the CLI

A small binary (~2MB), desktop only. It does not hold an identity, open
sockets, or synthesize anything. It writes to a local IPC socket and exits.
Invocation cost is single-digit milliseconds, which is what makes it usable
as an agent's notification channel.

If the node app isn't running, the CLI does not start it: it exits `NO_NODE`
and explains what it found. It does not claim to know *why* — a socket that
will not answer can mean nothing was started, or that something started and
has not bound yet, which on macOS is the ordinary case while a keychain prompt
waits. Autostart is the intended mitigation for the CLI depending on the app
(decision 5) and has not been built.

### `voicecast-node` — the Tauri v2 app

One codebase, five targets: Linux, macOS, Windows, Android, iOS. Long-running.
Holds the device identity, the iroh endpoint and its warm connections, the
allowlist, the playback queue, and the TTS engine. Exposes a local IPC socket
for the CLI on desktop.

On desktop it lives in the tray; the window is for configuration, not
operation. Starting at login is intended and not yet implemented, so today
something has to launch it.

**Every install is both sender and receiver.** There is no separate
"broadcaster" build. The CLI is desktop-only because that's where agents run,
not because phones can't send.

### `voicecast-core` — the shared Rust library

Everything that isn't UI: transport, discovery, pairing, allowlist, chunking,
queue, playback, engine abstraction. Compiles to all five targets. This is
where the complexity lives and where the tests go.

## Identity

Each install generates an **ed25519 keypair** on first launch. The public key
is the device's address (iroh `NodeId`, 32 bytes). Private key is stored in the
platform keystore — Keychain, Android Keystore, DPAPI, libsecret — falling back
to a `0600` file.

Display names ("Phil's Pixel", "desk") are **local labels only**. Identity is
always the key. Renaming never breaks pairing, and a name can't be used to
impersonate a device.

## Discovery

Three rungs, attempted concurrently, transparently upgrading to the best
available path:

| Rung | Mechanism | Works when |
|---|---|---|
| **Local** | mDNS / DNS-SD | Same network. ~1ms, no infrastructure, works offline. |
| **DNS** | pkarr signed records | Anywhere. Maps `NodeId` → current address. |
| **Relay** | n0 public relays | Both peers behind CGNAT and hole-punching fails. |

The DNS rung is what makes the UX good: **pair once, ever.** A phone can move
from wifi to LTE to a hotel network and stay reachable, with no static address
and no re-pairing.

## Membership

Discovery is not authorization. Anyone can find a node; membership is what
grants permission to speak on it.

**Pairwise pairing does not scale.** If authorization means "did I personally
pair with you," four devices in a full mesh is six pairings and five is ten.
For a tool whose main claim is easy setup, that is the wrong shape.

Instead devices join a **space**: a shared, signed roster of member NodeIds.
Joining is a one-time act with *any* existing member, so **N devices requires
N-1 joins**, and the invite can come from whichever device is in your hand.

### The roster

```
{ node_id, display_name, invited_by, signature, joined_at }
```

Authorization is "is this NodeId in my roster?" There is **no shared group
secret** — every connection is still individually encrypted with per-node
keys. The space is purely an authorization list, so compromising one device
leaks no key that decrypts anyone else's traffic.

### Joining

The inviter displays, the joiner scans. Terminals render QR codes in unicode
blocks and phones have cameras, so this direction needs zero typing.

```
  INVITER (any member)                        NEW DEVICE
  --------------------                        ----------
  $ voicecast invite
    |
    |- mint one-time token (5 min TTL, single use)
    |- build ticket = NodeId + relay hint + token
    |
    |- render QR in terminal ----------------->  [ scan ]
    |  (+ paste-able ticket as fallback)             |
    |                                                |
    |<---- QUIC connect, authenticated by NodeId ----|
    |      (token proves this was intentional)       |
    |                                                |
    |----- roster + capabilities ------------------->|
    |<---- node_id + chosen display name ------------|
    |                                                |
    |- safety code: 4821-9903                        |- safety code: 4821-9903
    |                                                |
    |- sign join record, gossip to space             |- store roster
    v                                                v
   joined                                          joined
```

**One payload, three presentations**, so the flow degrades gracefully:

- **QR code** — phones. The default path.
- **Paste-able ticket** — desktop-to-desktop, headless boxes, over SSH.
- **`voicecast://join/<ticket>` deep link** — tapping on the same device.

**The one-time token is not optional.** Without it, a QR photographed over your
shoulder — or sitting in terminal scrollback, or in a screen recording — is
permanent access to your speakers. It expires in five minutes and is consumed
on first use — and `rotate`, `leave` and `space leave` cancel one that is
still open, since all three change what it would admit somebody to.

**The safety code** is `hash(sorted(key_a, key_b))` truncated to 8 digits and
shown on both screens. Mostly redundant when a QR was scanned (the visual
channel already defeats a MITM), but it earns its keep when someone pastes a
ticket through a chat app.

### Why join records are signed

Members are frequently offline, and any member can invite. Suppose the iPhone
joins by scanning the Android's QR while the desktop is asleep. The desktop
wakes later and the iPhone connects — but the desktop has never seen this
NodeId.

The signature resolves it: the iPhone presents a join record signed by the
Android, the desktop already trusts the Android, so the iPhone is admitted.
Trust chains back to the space founder, who is in every roster. No online
authority is needed at any point.

### Roster sync

**Additions and revocations have completely different urgency**, and the design
follows that rather than treating them alike.

**Additions can be lazy.** The signed join record means a device vouches for
itself on arrival: the iPhone presents an entry signed by the Pixel, the
desktop trusts the Pixel, the iPhone is admitted — with no prior sync. Roster
sync is an *optimization* for additions, not a correctness requirement.

The corollary, which is easy to miss: **any current member may vouch for any
device**, so a device you no longer control is a device that could already
have added others. That is not a flaw in the check — it is what "a device
vouches for itself" means — but it is why the answer to a phone leaving your
hands is `rotate` rather than `revoke`. See decision 39.

**Revocations must propagate.** This is the entire reason sync exists. A
revoked device still holds a validly-signed join record, so a member that never
receives the tombstone will keep admitting a sold phone indefinitely.

The mechanism follows:

- **Roster digest in `Hello`** — a hash and entry count. Members connect anyway
  to send messages; matching digests cost nothing, mismatched ones trigger a
  roster exchange and merge.
- **Eager push on revoke** — actively dial every known member rather than
  waiting. The revoked device is told too, so a well-behaved node self-removes.
- **CRDT merge** — union of entries, union of tombstones, tombstones win.
  Conflict-free by construction; arrival order is irrelevant.
- **Re-adding** a revoked device works via a fresh join record signed later
  than the tombstone. Unforgeable, since it requires a real member to
  re-invite.

**No dedicated gossip layer.** `iroh-gossip` exists and works, but at 2–10
devices it is machinery without a job — no topic membership to manage, and
direct exchange on an already-open control stream fails in ways you can reason
about. Worth revisiting only if device counts grow past what full mesh handles.

### Revocation is eventually consistent

Stated plainly rather than glossed: a peer offline since a revocation was
issued will keep accepting the revoked device until it next syncs with any
member. Potentially days, if a tablet is switched off in a drawer.

**The escape hatch is space rotation, not faster revocation.** For a genuinely
urgent case — a stolen phone — create a new space and re-invite the surviving
devices. The stolen device is excluded immediately rather than eventually,
because it was never in the new space at all.

This is why the alternative was rejected. Short-TTL join records requiring
periodic re-vouching would bound the window, but they cost a persistent
re-vouching mechanism and make long-offline devices fail closed — a permanent
tax to fix a rare problem that rotation already solves in two QR scans.

### Two edges

**Name collisions.** Two devices both labelled `laptop` is a display problem,
not a correctness one — names are local labels. Disambiguate in the UI with a
NodeId prefix; `--to laptop` already errors on ambiguity.

**Clock skew.** Tombstone-versus-rejoin ordering uses timestamps, so badly
wrong clocks can misorder them. Not exploitable — the signature requirement
still holds — only confusing.

### Multiple spaces

A device may belong to several spaces at once, fully separated. The motivating
case is not "move a device between groups" but genuine overlap: a personal
phone should hear both work and home, while the work laptop hears only work.

**One keypair, many rosters.** A device keeps its single identity and holds one
roster per space. Authorization becomes "is this NodeId in *any* roster I
hold?", and every message carries the space it was sent in so the receiver
applies the right policy.

```
            desk ──┐
          laptop ──┤  space: work
                   │
           pixel ──┼── belongs to both
                   │
          iphone ──┤  space: home
           ipad  ──┘
```

Spaces never learn about each other. There is no cross-space roster sync, no
shared membership, and no way to address a device in a space you aren't in.

**Space labels are local**, exactly like device names. You may call it `work`
while another member calls it `team`. The space's real identity is a random
ID fixed at creation.

**Per-space receiver policy** is where this pays off: separate quiet hours,
separate mute, separate volume. Worth considering a **different voice per
space** — you'd know a message was work without listening to the words.

### Leaving a space

Leaving is two independent actions:

1. **Local removal** — drop the roster. Immediate, always works, offline or
   not.
2. **Self-revocation** — a signed tombstone gossiped to remaining members so
   they drop you. Best-effort; propagates as members reconnect.

Leaving never requires permission or connectivity. Other members can also
independently revoke you, which is the same mechanism from the other side.

Creating a new space on a device that already belongs to one needs no special
handling — it just gains another roster. "Detach and start fresh" is simply
leave-then-create, but neither step requires the other.

### One identity per device

Spaces hold **one person's own devices**. They are never shared with another
person — a deliberate scope boundary, not a missing feature.

That settles the identity model: **one keypair per device**, reused across
every space it belongs to. Per-space keypairs would buy isolation from someone
who isn't there, at the cost of multiple iroh endpoints per device.

It also removes a whole class of design work:

- "Any member can invite" is safe by construction — every member is you.
- No roles, permissions, or approval workflows.
- No per-device ACLs. Membership is the only authorization concept there is.
- Revoking retires your own lost or sold device; it never ejects a person.

The safety code on join still earns its keep — it defends a pasted ticket
against a man-in-the-middle — but it is no longer about verifying *who* is on
the other end.

## Message flow

A message is a **stream of chunks with an ID**, not a single blob. This is what
makes `cat file | voicecast` work — the receiver starts speaking sentence one while
sentence forty is still arriving — and it's what gives `voicecast stop` something to
address. Priority and queue behavior are specified in `cli.md`.

Chunking happens at sentence boundaries, which is required regardless: most TTS
engines degrade or fail on very long inputs, and sentence-level chunks are what
let playback start immediately.

## Security model

**Transport.** iroh is QUIC, so every connection is TLS 1.3 authenticated
against the node keypairs. End-to-end encrypted by construction, tied to the
exact NodeId you paired with.

**What a relay can see:** that two NodeIds exchanged data, when, and roughly how
much. **Never the text.**

**Authorization** is a per-receiver allowlist of NodeIds. An unpaired node
cannot make your device speak, and pairing requires physical possession of a
QR code or ticket.

**Abuse surface worth remembering:** anything an agent can be convinced to
print, it can be convinced to say. Rate limiting, quiet hours, and a mute
toggle are mitigations, not afterthoughts.

## Platform matrix

This table describes what is built, not what is planned. Where a row names
something that does not exist yet, it says so.

| Platform | TTS engine | Background listening |
|---|---|---|
| Linux | Piper, bundled in the Flatpak; espeak-ng floor only outside it | Tray |
| macOS | Piper, bundled in the `.app`; espeak-ng floor only if installed | Tray |
| Windows | Piper, placed by `cargo xtask piper`; no floor engine | Tray |
| Android | `android.speech.tts.TextToSpeech` | Foreground service |
| iOS | **None.** Builds, but reaches no engine — see below | **Foreground only** — OS restriction |

**Every desktop speaks through Piper**, which is the point: a message sounds
the same wherever it lands. Neural, CPU-only, no API key, ~20–60MB per voice
model. There is no universal native engine on Linux and espeak-ng sounds like
1994, so Piper started there and became the desktop answer everywhere.

**The espeak-ng floor is thinner than it looks.** `speech_engine()` falls back
to it on Unix, but nothing ships it: the GNOME runtime the Flatpak builds on
has no espeak at all, and the macOS bundle carries only Piper. So the floor
exists on a Linux host whose distribution happens to provide espeak-ng, and
nowhere else.

**What happens when Piper will not start** is worth knowing before reading a
`no_engine`. The app and the daemon pick engines in separate code —
`speech_engine()` in `app/src-tauri/src/lib.rs`, `fallback()` in
`crates/voicecast-daemon/src/main.rs` — and only Linux has a floor to fall to.

| | Piper missing, espeak present | Piper missing, espeak missing |
|---|---|---|
| `voicecastd`, Linux | speaks via espeak | refuses to start, naming both |
| `voicecastd`, macOS and Windows | — | starts silent, carrying Piper's reason |
| app, Linux | speaks via espeak | starts silent, carrying Piper's reason |
| app, macOS and Windows | — | starts silent, carrying Piper's reason |

**Every one of those reports Piper's own error**, because it is the part that
names something a person can act on — a missing dylib, a signature the OS
refuses, an Intel binary on arm64. Until #27 the two Unix rows did not: the
daemon gated on `unix` rather than Linux, so macOS refused to start over an
espeak floor it has never had and recommended an Arch package, and the app
replaced Piper's reason with "no speech engine is installed on this device",
which is false on any Mac where Piper is installed and merely broken.
Decision 30 records the split.

`voicecastd` on Linux still refuses rather than starting silent, and that is
deliberate: espeak-ng is a genuine floor there, so reaching this case means
the machine has nothing at all rather than one broken install.

**iOS reaches no engine at all.** It compiles — the five-target rule sees to
that — but it falls into the Unix branch of `speech_engine()`, where both
Piper and espeak-ng are spawned binaries that an iOS app cannot provide. The
result is `SilentEngine`, which reports `no_engine` honestly. Wiring iOS to
`AVSpeechSynthesizer` is the obvious way to fix that and is not yet written;
issue #4 tracks iOS never having been run at all.

**iOS cannot listen in the background.** This is an OS restriction, not a
framework limitation — Tauri, Flutter, and React Native all hit the identical
wall. Waking a backgrounded iOS app requires APNs, which requires a push
server. An iOS receiver only speaks while the app is in the foreground.

### The engine interface

```rust
trait SpeechEngine {
    fn speak(&self, chunk: &str) -> ...;
    fn voices(&self) -> Vec<Voice>;
    fn stop(&self);
}
```

Engine choice is a **receiver-side setting** — a direct consequence of the
text-only invariant. v1 ships `NativeEngine`; `ApiEngine` (ElevenLabs, OpenAI,
Azure — key stored on the receiver) can be added later without touching the
protocol.

Worth prototyping against: the `voicecast` Rust crate (Darilek) wraps native
synthesis across all five platforms behind one interface. If it holds up it is
most of `NativeEngine` for free.

One wrinkle: native OS engines generally play audio themselves, while Piper and
API engines hand back buffers. Routing everything through buffers costs a
little on native but gives uniform volume control, ducking, and a `stop` that
actually stops mid-sentence.

## Voice models and fallback

Only Linux has this problem — the other four platforms ship a usable native
engine. Linux needs Piper, and Piper needs a voice model: roughly 20–60MB of
ONNX weights.

**Neither bundling nor downloading works alone.** Bundling inflates every
package by ~60MB, invites a packaging fight over binary blobs, and — the
decisive one — **serves only English speakers**, since one bundled voice means
everyone else gets a tool that cannot speak their language. Downloading solves
all three but can fail on first run.

**So: both, tiered.**

1. **espeak-ng as the guaranteed floor.** Every distro ships it, so it is a
   package dependency rather than a bundled blob — no size cost, no packaging
   fight. It sounds like 1994, but it is intelligible and always present.
2. **Piper downloaded on first run**, and preferred the moment it lands.

The payoff is that **`no_engine` becomes nearly unreachable on Linux**: the
worst case is a robotic voice, not silence. And a voice picker serves other
languages, which bundling never could.

### Fallback must explain itself

A silent fallback is the real risk here — someone hears espeak, assumes that
is the product, and never discovers a better voice was one click away. So
degraded state is **visible and self-explanatory**, not merely logged.

```
  +- Voice ------------------------------+
  |                                      |
  |  (!)  Using fallback voice           |
  |                                      |
  |  Currently:  espeak-ng (robotic)     |
  |  Reason:     high-quality voice not  |
  |              downloaded yet          |
  |                                      |
  |  [ Download Piper - en_US - 63 MB ]  |
  |                                      |
  |  Speech works now. This only affects |
  |  how it sounds.                      |
  +--------------------------------------+
```

That last line matters as much as the warning: the device is working, not
broken, and the user should not go hunting for a fault.

**Reason codes**, carried in `Presence` and exposed to `--json`:

| Code | Meaning | Action offered |
|---|---|---|
| `not_downloaded` | First run; no model yet | Download |
| `downloading` | In progress | Progress, cancel |
| `download_failed` | Network, checksum, or host unreachable | Retry, choose mirror |
| `user_selected` | Deliberately chose espeak | None — this is intended |
| `unsupported` | onnxruntime unavailable on this platform | Explain; no action |
| `insufficient_disk` | Not enough space for the model | Free space, retry |

`user_selected` is why the state is *reported* rather than *nagged*: someone
who deliberately wants espeak should not be pestered forever.

### Visible from other devices too

Engine state travels in `Presence`, so `voicecast devices` shows it without visiting
the machine:

```
  NAME     PLATFORM   STATUS    VOICE                   LAST SEEN
  desk     linux      online    espeak-ng  (!)fallback  now
  laptop   macos      online    Samantha                now
```

### Distribution

Voice models come from Hugging Face — third-party infrastructure we do not
run, acceptable on the same logic as n0's relays. URLs are pinned with
checksums, with a mirror override in config.

**Deferred but a natural fit:** the devices are already a P2P mesh, and
`iroh-blobs` does content-addressed transfer. The first device could fetch a
voice from Hugging Face and the rest fetch it *from each other* — saving
bandwidth and working when the host is down, using infrastructure that already
exists. A v2 nicety, not a v1 requirement.

## Deferred

**Smart speakers (Google Home / Cast, Alexa).** Out of scope. Not being
designed for or around. One note kept only so the option isn't accidentally
closed off: a speaker can't synthesize from text, but that never has to mean
the *sender* does — a node on the speaker's LAN could render locally and hand
it audio. The text-only invariant survives if this is ever revisited.

**Cloud/API voices.** Anticipated by `SpeechEngine`, not built in v1.

**Self-hosted relay.** n0's public relays are acceptable. Config escape hatch
if that changes.

## State and durability

Nothing in this system requires cross-device backup. That falls out of the
design rather than being engineered, and it is worth recording *why*, because
the instinct is to build a recovery flow that turns out to preserve nothing.

| State | Lives | Survives |
|---|---|---|
| Identity keypair | System keyring | With its device. Meaningless elsewhere. |
| Space roster | Every member device | Meaningless without living peers. |
| Groups, defaults | `~/.config/voicecast/config.toml` | With your dotfiles. |
| Quiet hours, voice, volume | Each device, per space | With its device. |

### Why backing up a space is pointless

Consider losing every device at once — the only case where recovery could
matter. There are two branches and neither wants a backup:

- **If any device survives**, it already holds the roster. Nothing was needed.
- **If none survive**, the roster is a list of dead NodeIds. Restoring it
  yields nothing: those devices are gone, and new ones are not in it.

The roster has value only *relative to living devices*, so preserving it
independently preserves nothing. Recovery is `voicecast init` and a couple of QR
scans.

### Edge cases that resolve themselves

- **All devices asleep, want to add one** — inviting means using a device,
  which means turning one on.
- **Inviting while other members sleep** — the signed join record admits the
  new device retroactively.
- **Inviter drops mid-join** — the join fails; the ticket is valid until its
  TTL; retry.
- **Long-offline device returns** — digest mismatch on `Hello`, merge.

The one behavior worth adding: **leaving a space as its last member destroys
it**, and that should warn rather than happen silently.

## Design questions

None outstanding. Everything below was worked through and closed; the
remaining unknowns are implementation detail rather than architecture.

### Resolved

- ~~Queue and priority semantics~~ — three levels; `high` interrupts and
  resumes from the chunk boundary. See `cli.md`.
- ~~CLI surface~~ — see `cli.md`.
- ~~Feedback to the sender~~ — fire-and-forget by default; `--wait` and
  `--json` with per-target status and exit codes. See `cli.md`.
- ~~Quiet hours and mute~~ — receiver-side policy. The sender expresses
  intent and is told honestly what happened.
- ~~Linux voice models~~ — espeak-ng floor, Piper downloaded, fallback
  state surfaced in the UI.
- ~~Joining with no members online~~ — dissolves; see *State and
  durability*.
- ~~Roster sync~~ — digest in `Hello`, eager push on revoke, no gossip
  layer.
- ~~Text handling and chunking~~ — strict validation, protection-pass
  splitting. See `text.md`.
- ~~Wire protocol~~ — CBOR, stream-per-message. See `protocol.md`.
- ~~Multi-person spaces~~ — out of scope. Spaces are one person's own
  devices, which settles one-keypair-per-device.
- ~~Pairing at scale~~ — replaced pairwise pairing with a signed space
  roster. N-1 joins.
