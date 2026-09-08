# 62. `handle_peer` takes a trait, so the protocol can be driven by a test

**Status:** Accepted.

**Chosen:** a two-method `PeerConnection` trait in `transport.rs` — who is on
the other end, and the next stream pair — implemented for
`iroh::endpoint::Connection`. `handle_peer` becomes generic over it.

The receiving side of the protocol is sixteen message arms: every join,
revocation, speak decision and refusal a peer can trigger. It took a concrete
`iroh::endpoint::Connection`, which can only be obtained by binding an
endpoint and having a real second device dial it. So none of it had a test,
and every fix to it — including security fixes — said "verified by reading"
(#80).

**The change is smaller than it sounds, because the streams were already
abstract.** `read_msg` and `write_msg` have always been generic over
`AsyncRead` and `AsyncWrite`. Only the connection itself was concrete. Two
methods and one impl was the whole distance between "needs a second device"
and "needs a pair of pipes".

**`remote()` returns an `EndpointId`, not a string.** The first shape returned
a string, because most uses call `to_string()` anyway. That would have meant
widening the policy checks — `space_for`, `Roster::allows` — to take strings,
trading real type safety for a convenience a test does not need: generating a
key is one line. The compiler objected at five call sites, which was the right
answer.

**Deliberately not a wider abstraction.** This is not "a transport". It is the
two things one function asks for, named after what it asks for. A trait that
anticipated more would be a design nobody had tested either, which is the
problem being solved rather than a second instance of it.

**The first test it makes possible is the one that most needed it.** Decision
52 — a join request is signed for whoever is on the other end of the
connection, not for whoever the message names, because a ticket holder
enrolling a third key it does not hold leaves a member that revoking the
device in front of you does not remove. That check compiled and nothing ever
executed it. It now runs in 0.1s over two `tokio::io::duplex` pipes, and was
falsified by replacing the condition with `false` and watching the test fail.

**Costs.** The test still builds a `Node`, so it binds one endpoint — no
second device and no traffic, but not zero network either. `set_config_dir` is
a `OnceLock`, so the fake-connection tests share one scratch directory per
process and must not assume a clean one. And the fake yields exactly one
stream pair before ending, because `handle_peer` runs until the peer goes
away: a fake that kept yielding would hang rather than fail, which is the
worse failure for a test to have.
