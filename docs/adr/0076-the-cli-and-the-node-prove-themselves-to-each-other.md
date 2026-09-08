# 76. The CLI and the node prove themselves to each other

**Status:** Accepted.

**Chosen:** a 32-byte secret written to the config directory on every node
start, and a mutual challenge-response before the first request. The node
answers first. Both halves are portable, so `voicecast-core` gains no platform
conditional and the portability gate is untouched.

**What #54 was.** `handle_cli` read a `Request` and executed it with no check
at all, and `Request` is everything this device can be told to do: `Speak`,
`History`, `Invite`, `Join`, `Rotate`, `Revoke`, `SetMute`, `ClearHistory`,
`Quit`. On Linux the abstract namespace carries no permissions, so any other
user could run `voicecast invite` against your node and pair their phone into
your space permanently, or read everything your agents have said. Verified by
connecting to the socket from a process with no credentials and sending a
frame.

**Why the secret rather than a peer-uid check.** `interprocess` exposes
`peer_creds()`, and `euid()` on it is `cfg(unix)` — so a uid comparison puts a
platform conditional in the crate the portability gate exists to keep clean,
and does nothing on Windows, where the answer would be a named-pipe DACL
instead. Two mechanisms, two platforms, both needing exceptions. The secret is
one mechanism that works everywhere, and it reuses `store::write_private` and
`create_dir_private`, which already carry the only file-mode exceptions in the
project.

**And a uid check fixes only half of it.** It stops another user connecting.
It does nothing about another user *taking the socket name first*, which is
the macOS half of #54 — and having read `interprocess`'s source rather than
assuming, that half is worse than the issue claimed: `tmpdir()` returns a
hardcoded `/tmp/` on every Unix but Android, in a function whose own doc
comment calls it "the world-writable temporary directory". `$TMPDIR`, which is
per-user and 0700 on macOS, is consulted only on Android.

**The node answers first, and that is the whole design.** A client that spoke
first would hand its text, its invites and its history to whoever had taken
the name, and only then discover it was not the node. So the client sends a
nonce, the node returns a keyed hash of it, the client checks that before
sending anything, and only then proves itself in return. The two directions
carry different labels so neither answer can be replayed as the other.

**The error does not say "impostor".** A wrong token and a squatted socket are
indistinguishable from the caller's side, and the commonest cause is neither —
it is a `VOICECAST_CONFIG_DIR` that does not match the `VOICECAST_SOCKET` it
was given, which is exactly how a second node is run for testing. Asserting an
attacker would send someone hunting one.

**A connection that closes without speaking is a probe, not a refusal.**
`node_is_listening` and `bind_ipc` both connect and drop to tell a live node
from a dead one's leftovers, so without that distinction every app start would
print a warning about its own health check.

**What this does not fix, stated because a half-fix that reads as complete is
worse than none.** Another local user can still take the socket name before
the node does. Nothing leaks — that listener cannot prove itself and the CLI
refuses it — but the node does not start. Fixing that means a socket inside a
directory only the owner can enter, which changes the name-length budget in
decision 35, and is left as its own issue.

**blake3 is pinned to `no_neon`, and the five-target matrix is why this is
known.** Everything passed on Linux; `aarch64-apple-ios` failed at *link*
with `_blake3_hash4_neon` undefined, out of a C object blake3's build script
compiles on any little-endian aarch64. The portable Rust fallback is correct
and the inputs here are fifty bytes. It is the rule this project opens with,
collecting on a change that had nothing to do with platforms — a hashing
dependency, added for a security fix, that compiled everywhere and linked on
four targets out of five.

**Costs.** Two extra round trips on a local socket, which is microseconds
against the ~3ms startup the thin client is designed around. `blake3` and
`rand` become dependencies of the CLI, which had only `proto` and `text` — the
handshake is duplicated there for the same reason the framing and the socket
name already are, and the two copies are kept in step by hand. And a stale
token from a config directory that does not match the socket now fails loudly
where it used to work, which is the point.
