# 35. The socket is a name, and the node says so

**Status:** Accepted.

`voicecastd` printed `listening on /tmp/vc-d.sock` and bound
`/tmp//tmp/vc-d.sock`. Both are true from where the code sat and neither
helps: `VOICECAST_SOCKET` is a *namespaced name*, and what the platform does
with it differs completely.

| | where it lands |
|---|---|
| Linux | the abstract namespace — no file exists, under any path |
| macOS | `$TMPDIR` or `/tmp/`, prefixed onto the name |
| Android | `/data/local/tmp/`, likewise |
| Windows | a named pipe |

Because the CLI and the node apply the same mapping, they agree with each
other and disagree only with whoever goes looking. It is a lie that is
self-consistent, so nothing fails — `ls` on the logged path just returns
nothing, and the reader concludes the node is not running. That cost two
separate debugging sessions in one day. Issue #43.

**Why not print the resolved path instead.** That is the obvious fix and it
cannot be done here. The mapping is per-platform, `voicecast-core` may not hold
a `#[cfg(target_os)]`, and on Linux there is no path to print — the honest
answer on that platform is that the question does not apply. So the node stops
claiming a path rather than inventing one.

**Why a path-shaped value is a note and not an error.** `/tmp/vc.sock` works.
Refusing it would break setups that are running today, including the ones used
to test the join flow, to fix an aesthetic problem. One line at startup says
what will happen; nothing changes about what binds.

**Why the length error names the string.** `sun_path` is 104 bytes on macOS and
the platform's message named the limit but not what exceeded it — and what
exceeded it is not the value the caller set, because a prefix is added. The
error now carries the name and its byte count, so `206 bytes` against a limit
of 104 is a number you can act on rather than a puzzle.

**Cost.** One extra line on every startup. The startup output is now two lines
where it was one, which is the price of the second line being true.
