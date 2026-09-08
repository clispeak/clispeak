# 44. Exit codes and `--json` keep the promise the docs made

**Status:** Accepted.

The exit table in `docs/cli.md` has listed code 2, "no targets matched the
selector", since it was written, and the CLI never emitted it: every refusal
from the node arrived as code 1. `--json` was documented as working everywhere
and worked for three subcommands. Both were promises to an agent, which is the
only caller that reads either. Issue #66.

**Code 2 needed the node to say what kind of failure it was.** Everything
`resolve` reports is the selector matching nothing — a well-formed command
naming a device that is not here — which is a different thing to do about than
a command that is wrong, and matching on the message text to tell them apart
would break the first time someone reworded an error.

**The kind is a string, not an enum.** An unknown enum variant fails the whole
decode, so a newer node teaching this field a new value would break every
older CLI — the same trap that makes adding a `Response` variant unsafe, one
level down. An unrecognised string matches nothing, which is exactly what a
reader wants from a kind it has never heard of. There is a test for that case
specifically, because it is the one nobody would notice breaking.

**Fifty call sites became a constructor.** Adding a field to `Response::Error`
meant touching every construction of it, so they now go through
`Response::error` and `Response::no_target`. Worth doing on its own merits: the
kind is chosen at the point the failure is known rather than assembled by the
caller. It also means the next field costs one edit rather than fifty.

**A node that dies mid-request is code 5, not 1.** Once the socket is open, a
write or read failure is the node going away, and it was arriving as
"reading frame length: early eof" with code 1 — so an agent, correctly
following the table, went looking for a mistake in a command that was fine.

**`--json` now answers for every reply.** Three shapes stay hand-written
because the docs name their fields; the rest serialise as they stand, which
beats a table an agent has to scrape. `--quiet --json` also printed nothing at
all, because the JSON went through the same suppressed writer as the
narration: `--quiet` means do not narrate, `--json` means answer in JSON, and
they are not in conflict.
