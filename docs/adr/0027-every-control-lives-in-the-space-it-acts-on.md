# 27. Every control lives in the space it acts on

**Status:** Accepted.

**Decision.** Spaces is its own screen. Each space is a card holding its own
devices, and every control that acts on a space sits inside that space's card
and is named after it — `Leave main`, `Replace main`. Settings opens with this
device's name and holds what is set once and left alone; mute and quiet hours
moved there off the messages screen.

**Why.** Devices and spaces were separate lists in settings, so nothing said
which space a device belonged to or which space a button would act on. A
control named `Leave` sitting between two cards is ambiguous in exactly the
situation where being wrong is expensive.

**The constraint it exposes, and how it is shown.** Only the default space is
manageable: `accept_join`, `revoke` and `rotate` all take `spaces.current()`.
So a non-default card offers `Rename`, `Make default` and `Forget on this
device`, and says to make it the default first. The alternative was a button
that silently acted on a different space than the one it sat in, which is the
same class of failure as a confirmation dialog that never appears.
Tracked as issue #14 rather than worked around.

**`Forget on this device` rather than `Drop`.** Forgetting a space here does
not tell the other devices, which go on counting this one a member —
`leave_space` announces, `drop_space` does not. That difference existed only
in the code and now exists in the interface.

**How it was reviewed.** By rendering it. The real markup, script and compiled
CSS served against stub data and screenshotted headlessly, all three tabs. The
author's own run of that caught a wrapper `</div>` that had travelled with a
moved block, putting three sections outside the max-width container — with
tag counts still balanced, so nothing static would have found it.

A caution learned in the same review: a headless harness that clicks a tab on
a timer races the module that attaches the handler, and reports a mismatch
between the visible screen and the highlighted tab that does not exist in the
app. Drive the interaction from a module script that runs after the app's own,
not from a timer.
