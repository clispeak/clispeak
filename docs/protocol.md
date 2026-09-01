# Wire protocol

> Status: decided in shape, unspecified in detail. Field-level schemas will
> firm up during implementation.

## Encoding: CBOR

Binary, compact, serde-native, no codegen step — and critically,
**self-describing**.

That last property is doing real work. This project has an unusually bad
version-skew problem: a desktop updates whenever the package manager runs, an
iPhone updates whenever App Store review finishes *and* the user happens to
open the app. A desktop several versions ahead of a phone is the normal case,
not an edge case.

Self-description means an old peer can skip a field it has never heard of.
**Postcard was rejected for exactly this reason** — it is smaller and faster,
but an unrecognized field is a silent misparse rather than a skippable
unknown.

## Streams

QUIC gives cheap, independently flow-controlled streams. The design leans on
that hard rather than multiplexing everything down one channel.

**One bidirectional QUIC stream per message**, plus one long-lived control
stream per peer.

That single choice resolves three problems that would otherwise need designing:

**Cancellation** is a stream reset. `tts stop --id m_8fk2p` resets that
stream — no cancel message, no race between a cancel and chunk 40, no
ambiguity about how much was spoken.

**Priority isolation** is free. A five-minute document and an urgent
notification are separate streams with separate flow control. The long one
*cannot* head-of-line block the short one.

**Backpressure** is free. A receiver that reads only as fast as it synthesizes
stalls the sender through QUIC's own flow control. `cat war-and-peace.txt |
tts --to phone` cannot exhaust the phone's memory.

## Messages

### Control stream — long-lived, bidirectional

```
Hello        { node_id, display_name, platform, proto_version, capabilities }
RosterUpdate { entries[], tombstones[] }
Presence     { online, muted, quiet_hours_active,
               engine: { name, voice, tier, reason? } }
Ping / Pong
```

`Presence.engine.tier` is `full` or `fallback`; `reason` explains a fallback
(`not_downloaded`, `download_failed`, `user_selected`, …) so other devices can
show *why* without visiting the machine.

`capabilities` carries accepted content types, available voices, active engine,
and max chunk size — this is what lets new receiver kinds appear later without
a version bump.

### Message stream — one per message

```
  ->  SpeakBegin { msg_id, space_id, priority, voice_hint? }
  ->  Chunk      { seq, text }
  ->  Chunk      { seq, text }
  ->  SpeakEnd
  <-  Status     { state, detail? }
```

`Status` flows back on the *same* stream, which is what makes `--wait` and
`--json` work with no separate correlation mechanism. **The stream is the
correlation.**

States: `queued` → `speaking` → `spoken`, or terminally `muted`,
`quiet_hours`, `no_engine`, `rejected`, `cancelled`, `dropped`.

## Lifecycle

```
  sender                                    receiver
  ------                                    --------
  open stream
  SpeakBegin  ------------------------->    check roster membership
                                            check space policy
              <-------------------------    Status{queued}
  Chunk 0     ------------------------->    begin synthesis
              <-------------------------    Status{speaking}
  Chunk 1..n  ------------------------->    (flow-controlled by playback)
  SpeakEnd    ------------------------->
              <-------------------------    Status{spoken}
  stream closes
```

The sender may begin transmitting before it has read all of stdin — chunk 0
can be speaking while chunk 40 is still being read from the pipe.

## Versioning

`Hello` carries `proto_version`; both sides negotiate to the lowest common.
Additive fields are safe under CBOR. Removing a field or changing its meaning
requires a version bump.

The discipline this implies: **the core message set must be right early.** A
phone pinned to an old version has to keep working. Adding `Chunk.emphasis`
later is free; changing what `priority` means is not.

## Deliberate consequence

Stream-per-message commits us fairly deeply to QUIC. Cancellation, priority
isolation, and backpressure are all inherited from the transport rather than
built. A non-QUIC fallback transport would mean reimplementing all three by
hand — accepted, because iroh is the transport decision and it is QUIC.

## Not specified here

Chunk *content* rules — where text is split and how markdown, code blocks,
URLs, and abbreviations are handled — are a separate concern. See
`text.md`.
