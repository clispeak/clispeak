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

The app installs the command-line tool onto the host PATH itself, on first
launch, rather than offering it behind a button. The tool is the whole point
of the node — it is how an agent reaches it — so leaving it uninstalled until
someone finds a control means `voicecast` is simply missing from the PATH.
The same pass rewrites the copy whenever its bytes differ from the bundled
one, which is what keeps a Flatpak update from leaving a stale CLI behind.

**Consequences.** Piper is looked up in several roots now — the user data
directory first, then `/app/share/voicecast`, then `/usr/share/voicecast` —
so a user-installed voice still wins over the bundled one. A tray that fails
anyway is caught rather than fatal, including panics, because the tray crate
panics on a missing library instead of returning an error.

A Flatpak install has its own data directory, so it does not inherit the
space of a non-Flatpak install on the same machine. The device identity does
carry over, because that lives in the system keyring. Devices therefore have
to rejoin after switching to the packaged build.

## 20. A space is identified by its founder, and a device may hold several

**Decision.** A space's id is `<founder endpoint id>:<founder joined_at>`, and
a device keeps a map of them with a local label each and one marked default.

**Why an id at all.** Two devices that share more than one space have to agree
which one a message belongs to, or membership of the family space would
authorise speaking in the work one.

**Why derived rather than generated.** Every member has to arrive at the same
id without being told, including devices whose rosters were written before
spaces existed. The founder is the one member whose entry vouches for itself,
so every member can compute the id from a roster it already holds — and the
founding timestamp separates two spaces founded by the same device, which is
exactly what rotating produces. Generating a random id would have needed a
migration that every device performed identically, which is not something a
peer-to-peer system can arrange.

**Consequences.** Adding the id to the wire is backwards compatible: it is
optional, and a message without one is placed by looking up where the sender
is already known from, which is unambiguous whenever two devices share a
single space. Labels are local, like device names, and must be unique on the
device because they qualify device names — `work/laptop` cannot mean two
things.

`all` is scoped to one space and there is deliberately no selector meaning
"everywhere". Crossing spaces has to be spelled out, because a work message
arriving on the family tablet is the failure separate spaces exist to prevent.

Per-space receiver settings — separate mute, quiet hours and volume for each —
are specified in `cli.md` but not built. Policy is currently per device.

## 21. A stale socket is reclaimed only after a connection is refused

**Decision.** When binding the local socket returns `AddrInUse`, the node
tries to *connect* to it. A refused connection means the name outlived the
process that held it, and only then is it overwritten.

**Why this needed deciding at all.** `ipc.rs` said the socket is namespaced so
cleanup is not our problem. That is true on Linux, where the abstract
namespace discards the name with the process, and false everywhere else. Off
Linux the name is a file: a crash or a `kill -9` leaves it behind, every node
started afterwards fails to bind, and every CLI call reaches the corpse and is
refused — so the tool reports "no voicecast node is running" with the app
plainly on screen. Nothing recovers from that without deleting a file whose
location the design deliberately never mentions.

**Why not simply overwrite.** `interprocess` will displace a listener that is
still accepting connections, so overwriting on `AddrInUse` alone would let a
second node quietly steal the socket from a running one, and the CLI would
reach whichever won. That is a worse failure than the one being fixed, and a
silent one. Connecting is the only test that distinguishes a live node from a
dead one's leftovers.

**Consequences.** No platform conditional, so the portability rule still
holds — and the fix applies on Linux too, where the bug was merely unreachable
rather than absent. There is a narrow race between the refused connection and
the overwrite; a lock file would close it and would be worse, because a stale
lock file is the same class of bug one level further down.

This was found by running macOS for the first time, which is what the
compile-for-five rule was always meant to make cheap. The rule held: the
defect was in a portable crate and had been latent everywhere.

## 22. Windows refuses to start rather than starting silent

**Decision.** `voicecastd` on a platform with no speech engine fails at
startup with a message naming the app as the node there, instead of falling
back to a silent engine.

**Why.** A node that accepts messages and says nothing reports `queued` to the
sender and then does nothing at all. The sender has no way to tell that from
working, which is the exact failure the whole design avoids elsewhere by
sending reasons back rather than swallowing them.

**Cost.** Anyone testing IPC on Windows has no daemon to talk to. That is the
right trade while no Windows engine exists: the app links `voicecast-core`
directly and is the node on every desktop anyway.

## 23. The agent skill lives in this repo, and a test keeps it honest

**Decision.** `skills/voicecast/SKILL.md` is versioned here, and
`crates/voicecast-cli/tests/skill.rs` fails if it names a command or flag the
binary does not have.

**Why in-repo.** The skill is installed into agents and describes this CLI. A
copy kept anywhere else drifts the moment a flag is added, and "we will keep
it updated" is not a mechanism.

**Why a test rather than a note.** A skill that has drifted is worse than no
skill: it states with confidence that a flag exists, and the agent obeys it.
Generating the skill from clap would remove drift entirely, but most of its
value is judgement — when speaking is worth doing, what a `muted` reply means
— and judgement cannot be generated. Checking the generated-checkable part is
the useful half of that trade.

**What the skill carries that `--help` cannot.** That every message should
name the user and the agent, because a person with several agents cannot
otherwise tell whose voice is in their pocket. That exit 4 with `muted` is a
decision to respect, not a failure to retry or route around. That the failure
worth avoiding is not a missed message but being muted, after which every
later message is missed too.

**Consequences.** The skill establishes a working agreement with the user once
and instructs the agent to record it, so the preferences survive the session
that set them. Changing the CLI now means changing the skill in the same
commit, which is the point.

## 24. The core owns where state lives; the app only overrides it on mobile

**Decision.** `set_config_dir` is called by the app on mobile only. On desktop
the core's own config directory is used, and anything an older build left in
the app's data directory is moved across once.

**Why.** The override existed for a real reason — mobile has no XDG config
directory and `ProjectDirs` returns nothing there — but it was not gated, so
it also applied on desktops, where the directory exists and `voicecastd` was
already using it. One device ended up with two rosters, two histories and two
mute settings, sharing only the identity, because that lives in the keyring.
The symptom is baffling from outside: `voicecast devices` gives different
answers depending on which node happens to be up, with the same device id in
both.

**Why a move rather than a redirect.** Leaving the data where it was and
pointing the daemon at it would mean the daemon computing a path that belongs
to Tauri, which changes when Tauri decides it does. The config directory is
the core's own business, and the core is the thing both processes link.

**Why files already at the destination are set aside, not skipped.** Skipping
would let a stale copy win — the machine that found this had a roster written
by a daemon before the app existed, sitting exactly where the live one was
about to land. Overwriting would destroy it. Renaming it to `.superseded`
keeps both, and being wrong here costs someone every device pairing they have.

**Consequences.** An allowlist of filenames is moved, not the directory: the
app's data directory also holds a web view's caches, which are its own
business. The move is reported at startup rather than done quietly, because it
touches the file that holds every pairing. It is idempotent by construction —
a moved file is gone from the old place — and was verified against a live
three-device roster on Linux before shipping.

## 25. The device that speaks decides how long a caller waits

**Decision.** `--wait` is bounded by an estimate the *receiving* device makes
from the text, its own engine rate and everything already queued — not by a
constant, and not by the sender. An explicit `--timeout` overrides it.

**Why not a constant.** A constant is wrong at exactly one length. The flat
120 seconds held until someone sent 569 words, at which point the device spoke
all of it and the caller was told it had not finished.

**Why not a bigger constant.** This is the part worth keeping. On an M4,
Piper's `en_US-lessac-medium` produces **147.62 seconds of audio** for those
569 words, but takes **179.5 seconds end to end** — about 1.22x, because
chunks are synthesised one at a time and each pays its own start-up. Raising
the limit to 180, which is what measuring the audio alone would suggest, would
have failed again on the same message on the same machine by half a second.

**Why the receiver.** The sender knows the words and nothing else that
matters. The engine, its rate, and the queue in front of the message all live
on the device that will speak it — and for a message arriving from a peer, the
sender could not have known them at all.

**The assumed rate is 100 wpm**, well under any engine here — Piper measures
197–231, espeak-ng defaults to 175. The estimate bounds how long a caller
*may* wait, not how long it does: waiting ends when speaking does, so
over-estimating is free and under-estimating is the entire defect.

**Three shapes, all measured rather than argued:**

| | Measured |
|---|---|
| A long message | 569 words: 147.62s audio, 179.5s end to end |
| A message queued behind one | 14 words reported `speaking` at 120.4s, having not yet been spoken at all |
| A message being spoken | counted for nothing: 40 chunks playing, `depth() == 0`, `pending_words() == 0` |

The third was found in review, because a doc comment claimed the message being
spoken was counted whole and the code walked only the queues — from which the
thread removes a job in order to speak it.

**Consequences.** The estimate is made where the speaking happens, so it only
takes effect once the *receiving* device carries this. Until then `--wait`
against an older device still cuts off at 120 whatever the sender runs. That
belongs in release notes.

Fixing this also surfaced that `--timeout` never crossed the wire at all: it
was IPC-only, so `--timeout 600 --to Phone` did nothing whatsoever. It travels
on `SpeakBegin` now, optional and defaulted, so an older peer sending nothing
reads as expressing no preference rather than asking for zero.

## 26. The frontend asks for confirmation in the page, not through the webview

**Decision.** Destructive actions confirm through an in-page dialog. A gate in
`cargo xtask portability` fails the build if `confirm`, `alert` or `prompt`
appears in the frontend.

**Why.** `window.confirm` is not portable. WebKitGTK and WebView2 show it;
WKWebView shows a script dialog only if the host implements a `WKUIDelegate`
for it, and wry's delegate implements file upload, media capture permission
and new windows — no confirm panel. So `confirm()` returned false with nothing
displayed, and every action behind one returned at its first line while its
button reported success.

Five actions: clearing the history, removing a device, and dropping, leaving
or rotating a space. **Rotate is why this is more than a defect.** It is the
answer to a stolen device, and someone could believe they had locked one out
of a space while nothing whatsoever had happened.

**How far it reaches.** macOS and iOS, which share that backend — iOS has
simply never been run. Android is unaffected: wry's `RustWebChromeClient`
overrides `onJsConfirm` and builds a real dialog. Linux and Windows were
always fine, which is exactly why this survived so long: it was invisible on
the two platforms getting daily use.

**Why not `@tauri-apps/plugin-dialog`.** It would add a dependency, a
capability entry and a bundler step, to arrive at a dialog whose behaviour
still varies by platform. The in-page one is identical on all five.

**Why a mechanical gate rather than a note.** The symptom is a button that
looks like it worked. Nothing fails, nothing logs, and the only way to notice
is to check afterwards whether the thing happened — which is precisely what
nobody does with a confirmation dialog. The rule is the same one
`voicecast-core` already has one layer down: a platform assumption in shared
code, caught by a check rather than by a reader.

## 27. Every control lives in the space it acts on

**Decision.** Spaces is its own screen. Each space is a card holding its own
devices, and every control that acts on a space sits inside that space's card
and is named after it — `Leave main`, `Replace main`. Settings opens with this
device's name and holds what is set once and left alone; mute and quiet hours
moved there off the messages screen.

**Why.** Devices and spaces were separate lists in settings, so nothing said
which space a device belonged to or which space a button would act on. A
control named `Leave` sitting between two cards is ambiguous in exactly the
situation where being wrong is expensive.

**The constraint it exposes, and how it is shown.** Only the default space is
manageable: `accept_join`, `revoke` and `rotate` all take `spaces.current()`.
So a non-default card offers `Rename`, `Make default` and `Forget on this
device`, and says to make it the default first. The alternative was a button
that silently acted on a different space than the one it sat in, which is the
same class of failure as a confirmation dialog that never appears.
Tracked as issue #14 rather than worked around.

**`Forget on this device` rather than `Drop`.** Forgetting a space here does
not tell the other devices, which go on counting this one a member —
`leave_space` announces, `drop_space` does not. That difference existed only
in the code and now exists in the interface.

**How it was reviewed.** By rendering it. The real markup, script and compiled
CSS served against stub data and screenshotted headlessly, all three tabs. The
author's own run of that caught a wrapper `</div>` that had travelled with a
moved block, putting three sections outside the max-width container — with
tag counts still balanced, so nothing static would have found it.

A caution learned in the same review: a headless harness that clicks a tab on
a timer races the module that attaches the handler, and reports a mismatch
between the visible screen and the highlighted tab that does not exist in the
app. Drive the interaction from a module script that runs after the app's own,
not from a timer.

## 28. An invite names its space, and joining one adds rather than replaces

**Decision.** A ticket carries the space it invites into. `accept_join`
honours that rather than whichever space happens to be default when the
joiner arrives. `revoke` and `rotate` take a space too. And joining a space
*adds* it, except when the space it would displace holds only this device.

**Why the ticket carries it.** Deciding at the moment the joiner arrives
answers a different question from the one the person pressing the button was
answering. Change the default between showing a QR code and it being scanned,
and the device lands somewhere nobody chose.

**Why joining had to stop replacing.** `do_join` called `replace_current`,
with a comment saying joining means adopting a space's membership rather than
blending it. That was right when a device held exactly one space and became
wrong the moment it could hold several — a device in `home` that joined `work`
**lost `home`**. So multiple spaces could only be created locally and never
joined into, which is most of what anyone would want them for. Mine, from
decision 20, and not revisited when the assumption under it changed.

**The exception is the space every node founds for itself.** A fresh device
holds one space containing only itself. Joining should displace *that* —
otherwise a first pairing leaves an abandoned empty space beside the real one.
`Spaces::current_is_unshared` draws that line, and is tested for the case that
matters: a space with somebody else in it is never displaced.

**A joined space needs a local name**, because labels are local and the space
carries none. Numbered — `space-2` — rather than guessed from the inviter,
since a guess that collides is worse than a placeholder, and the response says
how to rename it.

**Unknown space names are an error, not a fallback to the default.** Acting on
the wrong space is precisely the failure a per-space button would otherwise
produce, and it is the reason the interface refused to offer those buttons
until this landed.

## 29. A space may be quieter than its device, never louder

**Decision.** `policy.json` holds one device policy and a map of per-space
overrides on top of it. Both get a say and either can refuse, so an override
adds silence and can never remove it. Muting the device mutes every space;
muting a space leaves the device and the other spaces alone.

**Why a floor rather than a replacement.** The alternative — an override that
supersedes the device policy — makes the mute switch conditional. Someone who
mutes their device before a meeting would still be spoken to by any space
holding an override, and would have no way to know which spaces those are from
the control they just used. That is a worse failure than the one this feature
fixes: a device that stays quiet when asked is the single promise the whole
policy layer makes. "Mute" that works most of the time is not a mute.

**What it costs.** There is no way to let one space through while the device is
muted, and no way to be awake for `home` at 23:00 while the device's own quiet
hours run 22:00–07:00. Someone who wants that keeps the device policy empty and
sets a window on each space instead — explicit, and visible in the interface,
rather than a rule that silently un-silences. `high_breaks_through` remains the
mechanism for "reach me anyway", which is what it was built for.

**Quiet hours are the union, for the same reason.** Two daily windows cannot
always be expressed as one — 09:00–10:00 together with 20:00–21:00 is two — so
the combination is not computed at all. Each policy is asked in turn and quiet
on either counts as quiet. That also keeps `Policy::verdict` untouched and its
tests meaningful.

**Locally-typed speech is governed by the device policy alone.** Text this
device originates is not *in* a space, so `Outgoing::space` is `None` for it and
only the device floor applies. Muting `work` must not silence the agent running
on the work laptop — that is what muting the device is for — and the history
records the space a message arrived in so a per-space refusal can be read back
to the space it applied to.

**An override outliving its space is dropped, not shown.** `policy_response`
skips overrides whose space this device no longer holds, and rotating or
leaving forgets them, because a space founded later could otherwise inherit
silence nobody asked for.

