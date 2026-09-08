# 115. A rename stamps itself strictly after the label it replaces

**Status:** Accepted.

**Chosen:** `Roster::rename` sets `renamed_at` to
`now().max(existing.saturating_add(1))` rather than `now()`.

**Why.** `renamed_at` is unix *seconds*, `merge` requires an arriving label to
be strictly newer than the one held, and a new member's `renamed_at` starts
life equal to its `joined_at`. So a rename in the same second as the record it
replaces tied, lost, and stayed lost — nothing retries a roster sync, so the
far device went on using the old name until some later rename happened to land
in a different second.

Found by the first test written against two real nodes (decision 114): pair
two devices, rename one, send a message, ask the other what it calls the
first. It answered with the old name.

**The tell was already in the suite.** `a_rename_survives_a_merge_with_a_stale
_peer` and `an_older_label_does_not_overwrite_a_newer_one` each slept 1100ms,
and the sleep was the whole reason they passed. A test that waits a second to
avoid a tie is not covering the tie; it is holding the bug still and looking
away. Both sleeps are gone, and both tests fail without this change — which is
how it was verified rather than assumed.

**What it costs.** A device's own renames are ordered by a counter rather than
purely by the clock when several land inside one second. That is the honest
ordering anyway: a device is the authority on its own name, and the later of
its own two renames is the one it meant. Runaway is not possible in practice —
`merge` reads a stamp more than `MAX_SKEW` ahead as no rename at all, which
would take three hundred renames inside one second to reach.

**Who this reaches.** Anyone pairing a device and naming it in one breath,
which is every scripted or agent-driven pairing and a fair number of human
ones.
