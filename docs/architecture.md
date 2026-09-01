# Architecture

> Status: draft — design in progress. Sections marked **OPEN** are undecided.

## What this is

A command-line tool that lets an agent speak text aloud on one or more of your
devices:

```
$ tts "build finished"                 # speak on default target(s)
$ tts --to phone "needs your input"    # speak on a named device
$ cat CHANGELOG.md | tts --to desk     # long-form, piped
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

### `tts` — the CLI

A small binary (~2MB), desktop only. It does not hold an identity, open
sockets, or synthesize anything. It writes to a local IPC socket and exits.
Invocation cost is single-digit milliseconds, which is what makes it usable
as an agent's notification channel.

If the node app isn't running, the CLI autostarts it.

### `tts-node` — the Tauri v2 app

One codebase, five targets: Linux, macOS, Windows, Android, iOS. Long-running.
Holds the device identity, the iroh endpoint and its warm connections, the
allowlist, the playback queue, and the TTS engine. Exposes a local IPC socket
for the CLI on desktop.

On desktop it lives in the tray and starts at login; the window is for
configuration, not operation.

**Every install is both sender and receiver.** There is no separate
"broadcaster" build. The CLI is desktop-only because that's where agents run,
not because phones can't send.

### `tts-core` — the shared Rust library

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
  $ tts invite
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
- **`tts://join/<ticket>` deep link** — tapping on the same device.

**The one-time token is not optional.** Without it, a QR photographed over your
shoulder — or sitting in terminal scrollback, or in a screen recording — is
permanent access to your speakers. It expires in five minutes and is consumed
on first use.

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

### Sync and revocation

Roster changes gossip between members and merge as an add-only set with
tombstones for revocation.

**Revocation is eventually consistent**, and this should be stated plainly
rather than glossed: a peer that has been offline since a revocation was
issued may still accept the revoked device until it syncs. Acceptable for a
personal device mesh. It would not be for a multi-tenant system.

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
makes `cat file | tts` work — the receiver starts speaking sentence one while
sentence forty is still arriving — and it's what gives `tts stop` something to
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

| Platform | TTS engine | Background listening |
|---|---|---|
| Linux | Piper (bundled, neural, CPU) | Tray + autostart |
| macOS | `AVSpeechSynthesizer` | Tray + autostart |
| Windows | WinRT `SpeechSynthesizer` / SAPI5 | Tray + autostart |
| Android | `android.speech.tts.TextToSpeech` | Foreground service |
| iOS | `AVSpeechSynthesizer` | **Foreground only** — OS restriction |

Four of five platforms ship a usable native engine for free. **Linux is the
gap** — there is no universal native engine, and espeak-ng sounds like 1994.
Piper is bundled there instead: neural, CPU-only, no API key, ~20–60MB per
voice model.

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

Worth prototyping against: the `tts` Rust crate (Darilek) wraps native
synthesis across all five platforms behind one interface. If it holds up it is
most of `NativeEngine` for free.

One wrinkle: native OS engines generally play audio themselves, while Piper and
API engines hand back buffers. Routing everything through buffers costs a
little on native but gives uniform volume control, ducking, and a `stop` that
actually stops mid-sentence.

## Deferred

**Smart speakers (Google Home / Cast, Alexa).** Out of scope. Not being
designed for or around. One note kept only so the option isn't accidentally
closed off: a speaker can't synthesize from text, but that never has to mean
the *sender* does — a node on the speaker's LAN could render locally and hand
it audio. The text-only invariant survives if this is ever revisited.

**Cloud/API voices.** Anticipated by `SpeechEngine`, not built in v1.

**Self-hosted relay.** n0's public relays are acceptable. Config escape hatch
if that changes.

## Open questions

- **OPEN** — Wire protocol format and framing.
- **OPEN** — Chunking rules at the edges: abbreviations, code blocks, URLs,
  lists, and other text that has no clean sentence boundary.
- **OPEN** — Roster gossip mechanics. `iroh-gossip`, or direct exchange on
  connect?
- **OPEN** — What happens when a space has no members left online and a new
  device wants to join.
- **OPEN** — Voice model distribution on Linux. Bundle a default with the
  package, or download on first run?

### Resolved

- ~~Queue and priority semantics~~ — three levels; `high` interrupts and
  resumes from the chunk boundary. See `cli.md`.
- ~~CLI surface~~ — see `cli.md`.
- ~~Feedback to the sender~~ — fire-and-forget by default; `--wait` and
  `--json` with per-target status and exit codes. See `cli.md`.
- ~~Quiet hours and mute~~ — receiver-side policy. The sender expresses
  intent and is told honestly what happened.
- ~~Multi-person spaces~~ — out of scope. Spaces are one person's own
  devices, which settles one-keypair-per-device.
- ~~Pairing at scale~~ — replaced pairwise pairing with a signed space
  roster. N-1 joins.
