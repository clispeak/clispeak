# 114. Two nodes in one process, so the protocol can be tested at all

**Status:** Accepted.

**Chosen:** `Node` takes its network as a `dyn Network` trait object and
carries its own config directory, replacing a concrete `Transport` and a
process-wide `OnceLock`. A test-only `loopback` module carries frames between
nodes in memory.

**Why.** Every pairing, roster sync, revocation and spoken message in this
project's history was verified by a person doing it by hand (#80). The cost
was not the typing. It was that a four-device space dissolving itself took a
day to explain, because there was no way to reproduce a multi-node roster
merge except with four real devices in front of you (#166).

Two things stood in the way and both are gone.

The first was the connection. `handle_peer` took an
`iroh::endpoint::Connection`, obtainable only by binding an endpoint and
having a real device dial it. `PeerConnection` had already solved half of it —
the accepting half — and its own comment said a wider abstraction would be "a
design nobody had tested either". That was right when it was written. What
changed is that a test with two real nodes needs the *dialling* half too, so
`Wire` has a third method. It is not anticipation; it is the other direction
of the same conversation, added when something needed it.

The second was quieter and worse. `identity::config_dir()` is a `OnceLock`,
so the first caller wins and every later one silently gets the first one's
directory. Two nodes in one process therefore shared an identity, a roster, a
policy and an outstanding invite while believing they were strangers. The
existing peer tests worked around it with a process-wide mutex and one
directory per *process*, and the comment above that mutex says what it cost:
adding a second test made it fail in the suite and pass on its own.

**What it costs.** `Node` holds a trait object rather than a concrete type, so
its network calls are dynamically dispatched — once per connection and once
per stream, against operations that cross a network, which is not a cost worth
measuring. A generic parameter would have avoided even that and would have
reached `Node`, the app, the daemon and every function between them to express
something only a test cares about.

**What it does not buy.** The loopback is not a simulation of the transport.
It models no latency, no loss, no reordering, no hole punching and no peer
that vanishes mid-stream. NAT traversal, doze and network switching stay
tested on hardware, and `CLAUDE.md` still says so. What these tests prove is
that the same `PeerMessage` frames move through the same `handle_peer`,
`do_join`, `sync_roster` and `send_to_peer` and produce the right state on
both devices.
