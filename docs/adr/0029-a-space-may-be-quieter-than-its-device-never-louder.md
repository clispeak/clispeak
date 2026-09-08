# 29. A space may be quieter than its device, never louder

**Status:** Accepted.

**Decision.** `policy.json` holds one device policy and a map of per-space
overrides on top of it. Both get a say and either can refuse, so an override
adds silence and can never remove it. Muting the device mutes every space;
muting a space leaves the device and the other spaces alone.

**Why a floor rather than a replacement.** The alternative — an override that
supersedes the device policy — makes the mute switch conditional. Someone who
mutes their device before a meeting would still be spoken to by any space
holding an override, and would have no way to know which spaces those are from
the control they just used. That is a worse failure than the one this feature
fixes: a device that stays quiet when asked is the single promise the whole
policy layer makes. "Mute" that works most of the time is not a mute.

**What it costs.** There is no way to let one space through while the device is
muted, and no way to be awake for `home` at 23:00 while the device's own quiet
hours run 22:00–07:00. Someone who wants that keeps the device policy empty and
sets a window on each space instead — explicit, and visible in the interface,
rather than a rule that silently un-silences. `high_breaks_through` remains the
mechanism for "reach me anyway", which is what it was built for.

**Quiet hours are the union, for the same reason.** Two daily windows cannot
always be expressed as one — 09:00–10:00 together with 20:00–21:00 is two — so
the combination is not computed at all. Each policy is asked in turn and quiet
on either counts as quiet. That also keeps `Policy::verdict` untouched and its
tests meaningful.

**Locally-typed speech is governed by the device policy alone.** Text this
device originates is not *in* a space, so `Outgoing::space` is `None` for it and
only the device floor applies. Muting `work` must not silence the agent running
on the work laptop — that is what muting the device is for — and the history
records the space a message arrived in so a per-space refusal can be read back
to the space it applied to.

**An override outliving its space is dropped, not shown.** `policy_response`
skips overrides whose space this device no longer holds, and rotating or
leaving forgets them, because a space founded later could otherwise inherit
silence nobody asked for.
