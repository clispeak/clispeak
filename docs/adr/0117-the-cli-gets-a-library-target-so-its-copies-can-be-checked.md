# 117. The CLI gets a library target, so its copies can be checked

**Status:** Accepted.

**Chosen:** `clispeak-cli` gains a `[lib]` target exposing `config`, `frame`
and a new `mirror` module, plus a `[dev-dependencies]` entry on
`clispeak-core`. A new `tests/drift.rs` holds every hand-kept copy up against
its original.

**Why.** Three things in the CLI are copied from `clispeak-core` by hand: the
IPC framing and its frame cap, the socket name and config path with their
environment overrides, and the `PEER_CONNECT` bound. They are copies because
depending on the node's crate would pull iroh, its transitive dependencies
and the whole of the roster, queue and policy code into a binary whose entire
purpose is starting instantly. That trade is right and `CLAUDE.md` states it.

What the trade did not have was a bill. The copies were in step, nothing
could have said when they stopped being, and the symptom of drift arrives
somewhere else entirely — that is not a prediction. Forgetting the
environment override in the socket name made every command talk to the first
node on the machine, "which looked like two unrelated bugs". A `PEER_CONNECT`
shorter than the node's made a speak to an unreachable device report that the
*local* node had never answered, and tell the reader to restart a healthy app
(#151).

**Why a library target rather than a gate that greps.** A binary crate exposes
nothing, so no test could reach these at all — which is why this was still
open. Comparing the two by reading both files with a regex would be a check
that passes when the shapes match and the meanings have moved, which is the
category of answer this project already collects.

**What it costs.** Nothing that ships. Measured rather than asserted: the
release binary starts in **2.0ms** over 200 runs and contains **zero** iroh
strings, so `clispeak-core` is not linked into it. A dev-dependency is built
for the test target only. The lib adds a compilation unit and one line of
`use` to `main.rs`.

**What the test does not do.** It does not merge the copies. They stay two,
deliberately, and a failure here does not mean this crate is wrong — it means
the two are no longer the same. Read both, decide which is right, and change
the other.

**Falsified rather than assumed.** Seven drifts were introduced one at a
time — a different default socket name, an override ignored, a shorter
`PEER_CONNECT`, a smaller frame cap, a little-endian length prefix, a renamed
token file, an altered handshake label — and each was caught by the test that
claims to cover it.
