# 84. An error you cannot dismiss

**Status:** Accepted.

Errors in the app deliberately do not time out: a failure must not vanish
before it is read. The comment said so, and it was right.

What it did not have was any way to say *I have read it*. No close control, no
click handler, and the bubble sat inside a `pointer-events-none` wrapper — so
even a control would not have been reachable. The only thing that cleared an
error was the next `say()`, and on a phone the next action may never come. One
failure held the bottom of the screen, over the tab bar, for the rest of the
session. Reported from the phone: *"any time there is an error toast it never
goes away"* (#144).

**The fix keeps the pinning and adds the way out.** An error is now a
`<button>` rather than a `<p>`, with `pointer-events-auto`, an `aria-label`
that appends "dismiss" to the message, and a click that clears it. A real
control because a thumb, a keyboard and a screen reader all already know what
one is; a bespoke tap handler on a paragraph would have served only the thumb.

Confirmations are unchanged — they still fade after 3.5 seconds, because a
confirmation that has been read is only clutter.

**A probe, `app/tests/probes/errors.js`, asserts both halves**: that an
untouched error survives well past the confirmation timeout, and that pressing
it clears it. Four of its six checks fail against the old interface and the
pinning check passes on both sides, which is the point — the way out was added
without giving up what the pinning was for.

**Costs.** One more thing to press. And the probe found a second bug while
being written: `pointer-events-auto` computed as `none` because `styles.css`
was stale, which is the generated-CSS trap `CLAUDE.md` warns about, caught
here only because this probe asserts a computed style rather than behaviour.
