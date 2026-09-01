# Decisions

Why things are the way they are. Newest last.

## 1. Peer-to-peer, no hosted server

**Decision.** Devices connect directly. No backend to deploy or pay for.

**Why.** A hosted server is the single biggest ongoing cost and operational
burden for a tool this small, and it makes "easy to set up" a lie — someone
has to run it.

**Cost.** Pure P2P across the internet is not achievable without *some*
rendezvous. Mobile carriers are almost universally CGNAT, which defeats UDP
hole-punching. The honest framing is: no server *you* run, with third-party
relay fallback carrying ciphertext.

## 2. iroh as the transport

**Decision.** Build on iroh rather than raw mDNS + sockets or libp2p.

**Why.** Gives us all three discovery rungs (local, DNS, relay) in one library,
with public relay infrastructure we don't operate. Node identity is an ed25519
public key, so pairing is just exchanging keys, and QUIC/TLS authentication
falls out for free. Its pkarr-based DNS discovery is what enables pair-once —
devices stay reachable across network changes.

**Cost.** Third-party dependency on n0's relay and DNS infrastructure. They see
connection metadata, never content. Accepted.

**Rejected.** LAN-only (phones go silent when you leave the house). Assuming
Tailscale (requires prior setup, poor first-run experience).

## 3. Tauri v2 for the receiver app

**Decision.** One Tauri v2 codebase for Linux, macOS, Windows, Android, iOS.

**Why.** iroh is a Rust library, so with Tauri the core *is* Rust — transport,
pairing crypto, queue, and playback live in one native library compiled to
every target. No FFI bridge to maintain.

**Rejected.** Electron (desktop only — no mobile at all). Flutter (mature, but
would need the same Rust core reached through `flutter_rust_bridge` — a seam
maintained forever). Compose Multiplatform (JVM runtime on desktop).
.NET MAUI (Linux desktop is community-only). React Native (no real Linux).

## 4. Every install is both sender and receiver

**Decision.** One node type. No separate broadcaster and receiver builds.

**Why.** The plumbing is symmetric anyway, and it removes an entire axis of
"which thing do I install where" from setup. The CLI is desktop-only because
that's where agents run — not because phones can't send.

## 5. CLI is a thin client to the local node

**Decision.** `tts` writes to a local IPC socket and exits. The long-running
Tauri app owns the identity and connections.

**Why.** A short-lived process would pay full connection setup — endpoint
creation, discovery, hole-punching — on *every* invocation. That's 200ms–2s per
message, unusable for an agent firing off notifications. Through a warm socket
it's single-digit milliseconds.

**Cost.** The CLI depends on the app. Mitigation: autostart it if not running
(~1s cold, fast thereafter).

## 6. Text only, on the wire

**Decision.** Only text is transmitted. The receiver always synthesizes.
Audio never crosses the wire.

**Why.** ~50KB of text is five minutes of speech; the same audio is tens of
megabytes. It also deletes the audio codec, the streaming pipeline, and the
local HTTP server the Cast path would have needed. The only thing that ever
required sender-side synthesis was dumb speakers — and those can be served by a
bridge node that renders locally, preserving the invariant.

**Cost.** Voice differs per device. Accepted — each device's voice becomes a
purely local setting. Receivers with no working engine must report that
back rather than silently dropping the message.

## 7. Any message length

**Decision.** The CLI has no opinion about length. Short notifications and
piped documents use the same path.

**Why.** The agent's instructions determine what gets spoken; the tool
shouldn't second-guess it.

**Cost.** Forces a chunked streaming protocol rather than single-blob
messages, makes `stop` and message IDs mandatory rather than optional, and
raises real queue/priority questions (still **OPEN**).

## 8. Native TTS engines out of the gate

**Decision.** Ship with free, built-in, no-API-key synthesis. Pluggable
`SpeechEngine` so cloud voices can be added later.

**Why.** Nothing to sign up for, nothing to pay for, works offline.

**Cost.** Linux has no usable native engine, so Piper is bundled there —
~20–60MB per voice model. Voice quality varies across platforms.

## 9. A signed space roster, not pairwise pairing

**Decision.** Devices join a *space* — a shared roster of member NodeIds, where
each entry is signed by the member who invited it. Any member can invite.

**Why.** Pairwise pairing is quadratic: four devices in a full mesh is six
pairings, five is ten. Writing out the four-device setup walkthrough is what
exposed it. With a roster, N devices needs N-1 joins, and the invite can come
from whichever device is in reach rather than always the desktop.

**Why signed.** Members are often offline. If the iPhone joins by scanning the
Android's QR while the desktop is asleep, the desktop must later admit a NodeId
it has never seen. The signature chains that trust back to a member it already
knows, with no online authority involved.

**Deliberately not a shared group secret.** Every connection is still
individually encrypted with per-node keys. The roster is an authorization list
only, so compromising one device leaks nothing that decrypts another's traffic.

**Cost.** Revocation is eventually consistent — a long-offline peer may honor a
revoked device until it syncs. Fine for a personal mesh; not for multi-tenant.

## 10. Fire-and-forget by default, confirmation opt-in

**Decision.** `tts` returns as soon as the local node accepts the message.
`--wait` blocks for per-target terminal status; `--json` emits it structured.

**Why.** The caller is an agent that may fire many messages. Blocking on
playback by default would make it a latency source in someone's agent loop.
But an agent sometimes genuinely needs to know a message landed, so the
information has to be *available* — just not mandatory.

**Cost.** Two code paths, and exit codes have to mean different things
depending on `--wait`.

## 11. The receiver enforces policy; the sender only expresses intent

**Decision.** Mute, quiet hours, and volume are receiver-side and cannot be
overridden remotely. `--priority high` may interrupt a message, but whether it
breaks through quiet hours is a per-device toggle, off by default. Whatever the
receiver decided is reported back honestly.

**Why.** The alternative fails immediately: the first time an agent decides
everything is urgent, "urgent" stops meaning anything, and a device you muted
starts talking. Silent discards are equally bad — the agent must be able to
learn a message was suppressed.

**Related.** This is what makes `no_engine` a first-class status rather than a
silent failure — a Linux receiver with no voice model downloaded reports it.

## 12. Multiple spaces per device, one keypair

**Decision.** A device may belong to several fully separate spaces at once,
holding one roster per space but keeping a single identity. Messages carry
their space so the receiver applies per-space policy. Leaving is local removal
plus a gossiped self-revocation.

**Why multi-membership rather than move-between.** The real case isn't
relocating a device, it's overlap: a personal phone should hear both work and
home while the work laptop hears only work. A model that only supports moving
can't express that, and "leave and rejoin" would be a daily chore.

**Why one keypair.** Per-space keypairs isolate better but need multiple iroh
endpoints per device and multiply the key management. For a mesh of one
person's own devices the isolation buys nothing — the same NodeId appearing in
both of your own spaces is invisible to anyone.

**Settled: spaces are never shared with another person.** That removes the
only reason per-space keypairs were ever on the table, and with it a class of
design work — no roles, no permissions, no approval workflows, no per-device
ACLs. Membership is the only authorization concept in the system, and revoking
retires your own device rather than ejecting a person.

**No cross-space "all".** `--to all` is scoped to a single space; reaching
several requires naming them. The failure this prevents — a work message
arriving on the family tablet — is the entire reason someone made separate
spaces.
