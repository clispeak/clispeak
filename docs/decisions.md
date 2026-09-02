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

**Decision.** `voicecast` writes to a local IPC socket and exits. The long-running
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

**Decision.** `voicecast` returns as soon as the local node accepts the message.
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

## 13. CBOR, and one QUIC stream per message

**Decision.** CBOR encoding. One bidirectional QUIC stream per message, plus a
long-lived control stream per peer. Protocol version negotiated in `Hello`.

**Why CBOR over postcard.** Postcard is smaller and faster, but it is not
self-describing. This project has severe version skew by construction — a
desktop updates on a package manager run, an iPhone updates when App Store
review finishes and the user opens the app. Old peers must be able to skip
fields they have never heard of rather than silently misparsing them.
Envelope size is irrelevant anyway; the text payload dominates.

**Why stream-per-message.** It resolves three problems for free rather than
designing each: cancellation becomes a stream reset (no cancel/chunk race),
priority isolation becomes independent flow control (a long document cannot
head-of-line block an urgent notification), and backpressure becomes QUIC's
flow control (a receiver reading at synthesis speed stalls the sender, so
piping a huge file cannot exhaust a phone's memory).

**Cost.** It commits us deeply to QUIC. Any non-QUIC fallback transport would
mean reimplementing cancellation, prioritization, and backpressure by hand.
Accepted — iroh is the transport, and iroh is QUIC.

## 14. Reject bad text; don't silently rewrite it

**Decision.** The CLI validates before sending. Markdown and bare URLs are
**errors** carrying the offending spans, a concrete suggested rewrite, and a
named escape hatch. Emoji and stray whitespace are handled silently. Exit code
`6`, distinct from usage errors. `--strip` converts instead of rejecting;
`--raw` skips validation.

**Why, and why this is better than the original proposal.** The first design
was to normalize silently — strip markdown, summarize URLs, speak the result.
The caller is an agent, though, and **an agent can act on an error message**:
read it, fix the text, retry. That makes validation a working feedback channel
rather than an obstacle, which it would be for a human piping a file.

It also produces better speech. Text an agent *wrote to be spoken* beats
markdown mechanically stripped after the fact — "Updated 3 files" is a better
sentence than anything a rewriter extracts from `Updated **3 files**`.

**The line.** Error when there is an obvious correction the agent could make;
handle it quietly when there is not. Markdown has an obvious plain-prose
alternative. Emoji does not — an error on 🎉 would be a puzzle with no answer.

**Cost.** Breaks `cat CHANGELOG.md | voicecast`, since a changelog is markdown. That
now requires `--strip`, which makes the conversion explicit rather than
silent — an acceptable trade, and arguably clearer.

**Knock-on benefit.** With markdown rejected at the door, chunking is no
longer a rewriting problem. It reduces to finding safe sentence boundaries,
which is a much smaller and more testable job.

## 15. Lazy roster sync, eager revoke, no gossip layer

**Decision.** Roster digest in `Hello`; exchange and CRDT-merge on mismatch.
Revocations are actively pushed to every known member. No `iroh-gossip`.

**Why the asymmetry.** Additions are already handled by the signed join record
— a device vouches for itself on arrival, so sync is an optimization, not a
correctness requirement. Revocations are the opposite: a revoked device still
holds a valid signature, so a member that never receives the tombstone keeps
admitting it forever. The urgent path is deletion, not addition, which inverts
what a naive design would prioritize.

**Why no gossip layer.** At 2–10 devices, gossip is machinery without a job.
Direct exchange over the already-open control stream has no topic membership to
manage and fails comprehensibly. Revisit only if device counts grow well past
full mesh.

## 16. Space rotation instead of fast revocation

**Decision.** Revocation stays eventually consistent. The remedy for an urgent
case is `voicecast space rotate` — found a replacement space and re-invite the
surviving devices.

**Why.** Short-TTL join records with periodic re-vouching would bound the
exposure window, but they cost a permanent re-vouching mechanism and make
long-offline devices fail closed. That is an everyday tax to fix a rare
problem.

Rotation excludes the lost device *immediately* rather than eventually, since
it was never a member of the new space. It works only because joining is
cheap — a direct payoff from the N-1 join design.

**Cost.** Requires physical access to each surviving device to re-scan. For
three survivors that is two QR scans, which is an acceptable price for an
emergency path.

## 17. espeak-ng floor, Piper downloaded, fallback made visible

**Decision.** Linux depends on distro espeak-ng as a guaranteed floor and
downloads Piper on first run, preferring it once present. Fallback state is
shown in the UI with a machine-readable reason, and travels in `Presence` so
other devices see it too.

**Why not bundle the model.** ~60MB on every package, a packaging fight over
binary blobs, and decisively: one bundled voice serves only English speakers.
Everyone else would get a tool that cannot speak their language.

**Why not download-only.** First run would fail without network, and the
message would be lost rather than merely ugly.

**Why degrade quietly here, when text validation fails loudly.** Not an
inconsistency — the difference is who can act on the failure. A markdown error
reaches an agent that rewrites and retries in one round trip. A missing voice
model can only be fixed by a human who is not watching, so failing loudly
would just lose the message. **Loud failure is right when someone can respond
to it.**

**Why fallback must explain itself.** The real risk of a silent fallback is
that someone hears espeak, assumes that is the product, and never discovers a
better voice was one click away. So the UI states which engine is active, why,
what to do about it, and — equally important — that speech is working and no
fault needs hunting. `user_selected` is a reason code precisely so a
deliberate espeak user is reported rather than nagged.

**Cost.** `no_engine` becomes nearly unreachable on Linux, which is the point,
but it means quality varies silently unless the UI does its job.

## 18. Named `voicecast`

**Decision.** The project and its CLI are `voicecast`. Crates are
`voicecast-core`, `voicecast-cli`, `voicecast-proto`, `voicecast-text`,
`voicecast-engine`.

**Why "cast".** The distinguishing feature is not that it speaks — plenty of
tools do — but that it **fans out to many devices at once**. `--to all` is
literally casting. Broadcast semantics describe the product better than speech
semantics do.

**Why not `agentcast`.** Taken on crates.io by a dormant crate, and — the
decisive part — taken on npm by an *actively developed* AI-agent tool. A
near-identical name in the same niche would interleave in every search result.
`agent-cast` was free but a hyphen is thin separation from an active neighbour.

**Why not `agent-speak`.** Free on crates.io, but AgentSpeak(L) is an
established agent-oriented programming language with decades of literature
behind it. Bad for a name whose job is being findable.

**Why no `agent-` prefix at all.** It names the caller rather than the
function, and the tool is not agent-only — piping a file is in the design. The
README's first line carries that framing better than the name would, and it
will age better.

**Free on both crates.io and npm**, no PATH collision, no trademark.

## 19. The Flatpak bundles Piper and ships its own indicator library

**Decision.** The Flatpak carries a Piper binary and one voice in
`/app/share/voicecast`, and builds libayatana-appindicator as a module.

**Why.** The GNOME runtime has no speech synthesiser at all — not even
espeak-ng — so a sandboxed install without a bundled engine starts up mute,
and the "using a fallback voice" state the interface reports would have
nothing to fall back to. Bundling costs about 85MB and makes the app work the
moment it is installed.

The runtime also ships no appindicator library, which the tray needs. That
matters more than a missing icon: closing the window deliberately hides it so
the node keeps running, so with no tray there is no way back to the window
except `voicecast show` from a terminal.

Shipping the library is necessary but not sufficient. An indicator claims its
own bus name and then calls the watcher, and the sandbox forbids both by
default, so the manifest also grants `--own-name=org.kde.StatusNotifierItem-2-1`
(the id is the process id, which inside a sandbox is always 2) and
`--talk-name=org.kde.StatusNotifierWatcher`. Without those the icon silently
never appears, with no error anywhere.

**Consequences.** Piper is looked up in several roots now — the user data
directory first, then `/app/share/voicecast`, then `/usr/share/voicecast` —
so a user-installed voice still wins over the bundled one. A tray that fails
anyway is caught rather than fatal, including panics, because the tray crate
panics on a missing library instead of returning an error.

A Flatpak install has its own data directory, so it does not inherit the
space of a non-Flatpak install on the same machine. The device identity does
carry over, because that lives in the system keyring. Devices therefore have
to rejoin after switching to the packaged build.
