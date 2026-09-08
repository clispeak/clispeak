# 93. Something answering is not a node answering

**Status:** Accepted.

Any local user can take the socket name before the node does: Linux's abstract
namespace carries no permissions at all, and `interprocess` puts the macOS
socket in what its own source calls "the world-writable temporary directory".
Decision 76 accepted that and closed the disclosure half — the handshake means
a squatter cannot drive the node or receive what was meant for it.

**What was left was not the denial of service. It was the report.** Both
checks on the way to starting a node connected, saw something answer, and
concluded a node was running:

- `bind_ipc` said *"another node is already running"* — false;
- `node_is_listening()` returned true, so the app declined to start its own
  node and said the same thing.

So the node was down, and the reason on screen described a machine state that
did not exist. Somebody would have gone looking for a second app to quit
(#128).

**The handshake could always tell them apart.** A squatter does not have the
token and cannot produce the node's proof. It was simply never asked, because
both callers connected and dropped rather than speaking.

**The decision: ask.** `who_is_listening` returns `Nothing`, `Node` or
`Stranger(why)`, and both callers use it. A stranger is named as one, with the
reason it failed, and with `CLISPEAK_SOCKET` offered as the way round it.

**It has to run before the token is written**, and that is the ordering that
makes this work at all: `install_token` replaces the file on every node start,
so a node already running still holds the *old* secret. Probe afterwards and
every answer is `Stranger`, including a genuine second node — the check would
be exactly as wrong, in the other direction. So `serve` probes first and
installs second.

**`bind_ipc` therefore stops claiming to know.** It is reached only when
something took the name between the probe and the bind, and by then the token
is gone, so both possibilities are named: another node that started a moment
ago, or another local process. Its test now asserts that it does *not* claim
which.

**One second, not five.** The probe is bounded well inside
`HANDSHAKE_TIMEOUT`: a node on the other end of a local socket answers in
microseconds, and waiting the full handshake timeout would stall every app
start that met a squatter. Being too impatient is contained — the caller
starts a node, and `bind_ipc` refuses if one really was there.

**`node_is_listening()` is now false for a stranger**, deliberately. The
caller's next move is to start a node, which fails to bind and says what is
actually wrong. Declining to start and reporting a second node that does not
exist is the worse of the two.

**Costs.** A squatter still stops the node starting; this is the report, not
the protection. The OS-enforced half is the rest of #128, and Patrick settled
its open question on 3 September 2026: `CLISPEAK_SOCKET` **keeps meaning a
name**, resolved inside a private directory rather than pointing anywhere on
the filesystem, so `CLAUDE.md`, `docs/cli.md` and the README stay true.
