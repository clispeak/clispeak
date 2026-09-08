# 39. Any member may vouch for any device, and the docs now say so

**Status:** Accepted.

`roster.rs` opened by describing a rule its own code did not run: that a join
record is accepted only if its inviter is someone already trusted. That check
exists, in `Roster::admit`, which nothing outside the tests calls. What a
roster sync actually goes through is `Roster::merge`, which verifies every
signature and now every date, but not the inviter. Issue #49.

**Enforcing it would have bought nothing.** `merge` is reachable only from a
peer that `allows` has already confirmed is a current member of the space
being synced — both call sites are gated, one because we chose the peer from
our own roster and one by an explicit check that refuses a stranger. A current
member can sign a record for any endpoint id it likes with its own key, and
that record satisfies `admit` as readily as it satisfies `merge`. The rule
would only stop a *non-member* injecting records, and a non-member's sync is
refused before it reaches either function.

**It would also have cost something real.** The check's answer depends on
which records have already been merged, so two devices receiving the same
updates in a different order could disagree about the roster permanently —
giving up the convergence the whole add-only design exists for. And it would
make revoking a device silently orphan every device that device had invited,
which is a surprising way to lose a phone from a space.

**So the documentation moved, not the code.** The header now states the rule
as it runs — any current member may vouch for any device — and says what
follows from it: a device you no longer trust is a device that could have
added others before you revoked it, so `rotate` rather than `revoke` is the
answer when a device is out of your hands. That was already true and already
what `rotate` is for; it was simply never written next to the thing that makes
it necessary.

`adopt_id` was dead in the same way and had a real consequence, so it was
wired up rather than documented — see decision 37.
