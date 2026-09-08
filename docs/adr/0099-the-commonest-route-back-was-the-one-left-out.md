# 99. The commonest route back was the one left out

**Status:** Accepted.

Decision 98 made a record that beats a revocation clear it, in `admit` and on
the merge path. It missed `insert_self_signed`, which is the path an
**inviter** takes — and therefore the path `accept_join` takes every time a
device pairs.

So removing a device and pairing it again, which is the most ordinary
recovery there is, put it back as a member with its revocation still standing
beside it. That is exactly the half-state decision 98 exists to remove,
reachable by the commonest route to it, and the entry did not notice because
the two paths it fixed are the ones a *peer's* records arrive by.

Found by reading the code for what a re-pair would do, rather than by
re-pairing and looking afterwards — which matters, because doing it the other
way round would have written a fresh contradiction into a roster we had just
finished cleaning, and it would have looked like the fix failing.

**The removal is unconditional here**, unlike in `admit`, because this record
is signed `now` and no tombstone can be later than that: `merge_at` clamps a
future one to the moment it was heard.

**Costs.** None that are new. It is the rule 98 already stated, applied to the
third place that needed it.

**And the lesson is about how 98 was checked.** Its tests drove `admit` and
`merge`, because those are where the incident happened. Nothing drove the
inviter, so nothing failed. A fix verified against the incident it came from
covers the incident; covering the *rule* means asking which other code does
the same thing, and there were three answers, not two.
