# 69. Whoever takes focus away is who gives it back

**Status:** Accepted.

**Chosen:** three changes, each where the fault is. `withButton` restores
focus to the button it disabled. `openModal` treats `<body>` as the *absence*
of an opener rather than as one. `close()` releases focus out of the dialog
before hiding it.

**What #113 was.** Every dialog opened from a `withButton` button left focus
on nothing, so a keyboard or screen-reader user lost their place in the page
every time they confirmed anything — removing a device, clearing history,
installing the skill, the space dialogs.

`withButton` sets `disabled = true` synchronously, *before* the action runs.
Disabling the focused element blurs it to `<body>`. Only then does `ask()`
call `openModal`, which captured `document.activeElement` — `<body>`, not the
button. So `restoreFocus` succeeded on its first attempt at restoring focus to
nothing, and the retry never ran.

**Decision 59 named this mechanism and fixed the other half of it.** Its own
words: "the opener is usually a button `withButton` disabled for the duration
of its action, and a disabled element cannot take focus". That is the right
observation, and the fix it produced — retry until the button is focusable —
assumes the opener *is* the button and merely unfocusable for a while. It was
already `<body>` before `openModal` ever looked. So the retry waited patiently
for a condition that was already true about the wrong element.

**The probe had been failing on `main` and was read as a flake.** Its author's
note says it "failed on another machine and passed here", which is what a
genuine platform-independent bug looks like when only one person's timing
happens to expose the symptom. It was neither flaky nor local.

**Two faults sat on one line and each of us found one.** Replacing the single
`setTimeout(0)` with a bounded poll (#90) fixed a real defect — a single
macrotask is a bet on how long a re-enable takes, and it loses whenever the
action is slow. That fix made the probe green on one machine without touching
what was broken on another. Both are needed; neither substitutes for the
other, and the probe going green is what made it look as though one did.

**Why three changes and not one.** Any single one of them makes the probe
pass. `withButton` alone would leave `openModal` reporting a restore it did
not perform, and would leave focus sitting inside a hidden subtree whenever a
dialog was opened some other way. The rule that made the split obvious is that
whoever takes focus away is who gives it back: the button's own helper caused
the blur, so it repairs it, and the dialog took focus into itself, so it
releases it.

**Costs.** `withButton` now touches focus, which is a concern it did not have
before, and it has to be guarded so that anything deliberately taking focus in
the meantime keeps it. The check is `document.activeElement === document.body`
— narrow on purpose, since a broader "is focus somewhere useless" test would
start guessing. And all of this is verified only by a probe that needs a real
Chrome and is not run by CI, so it is a command someone runs and a pull
request that says whether they did.
