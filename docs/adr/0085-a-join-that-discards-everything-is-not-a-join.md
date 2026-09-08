# 85. A join that discards everything is not a join

**Status:** Accepted.

Decision 83 ended with a note that `adopt` deserved half the blame, filed as
#145. This is that half, and a second silence found beside it.

**What `adopt` did.** It verified each record and inserted the ones that
checked out. A record that did not verify was dropped with nothing said —
which is correct behaviour and no report. When the rename changed the signing
domain separator, a peer sent three members, all three were refused, and the
device announced *"joined space, 1 of 1 device"*. Both ends were internally
consistent and neither could explain the other, because neither had noticed
anything happen. **Discarding everything and being sent nothing are spelled
the same way.**

**The decision: make the caller name the discards.** `adopt` and `from_parts`
now return `(Roster, Vec<RosterError>)`. Not a log line inside them — a
returned value, so ignoring it is `.0` or a `let (_, _)`, which is a thing
somebody wrote on purpose rather than a thing nobody thought about. The node
prints what was refused and why on both paths that take one, a join and a
roster sync.

**And a join that kept no record of this device now fails.** `do_join` asks
whether the roster it just built holds `me`; if it does not, the join errors
rather than reporting a membership that does not exist. The message counts
what was offered and what was refused, and when every refusal is a signature
it says what that means — two devices on different builds — because that is
the one diagnosis nobody can reach from the evidence without being told.

The other end has already recorded the membership by then, so the two devices
disagree about whether the join happened. That is the same disagreement as
before; the difference is that now exactly one of them says so, and it is the
one in front of the person.

`merge` still drops quietly, deliberately: every roster reaching it has been
through `from_parts` already, so a second report would name the same record
twice.

**Costs.** A signature change in a public API, and four call sites in tests
that now say `.0`. Cheap against a day.
