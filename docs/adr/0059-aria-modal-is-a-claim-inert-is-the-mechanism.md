# 59. `aria-modal` is a claim; `inert` is the mechanism

**Status:** Accepted.

Four dialogs, each with the open/Escape/backdrop/close pattern hand-copied,
each carrying `role="dialog"` and `aria-modal="true"` — a promise that the
rest of the page is unreachable — and none of them keeping it. Tab walked out
of "Remove device?" into the tab bar behind the backdrop, where a
screen-reader user could activate the very thing the dialog was asking about.
Closing dropped focus to `<body>`. Two dialogs at once was reachable through
that Tab escape, and left the first one's promise unresolved for ever with the
button that opened it stuck reading "…". Issue #75.

One `modal.js` now holds it: `inert` on the dialog's siblings, a Tab trap,
focus restored to the opener, and a single-dialog guard that answers a second
`ask` with a cancel rather than stacking it.

**Not `<dialog>` with `showModal()`.** It would give the trap and the top layer
for free, but these panels are already styled as full-screen flex backdrops on
five platforms, and `::backdrop` plus the UA's own centring would have to be
undone on every one of them. The behaviour was what was missing, not the
markup.

**Focus restore retries once.** The opener is usually a button `withButton`
disabled for the duration of its action, and a disabled element cannot take
focus — so restoring synchronously left focus nowhere, which is the bug this
file exists to fix, reintroduced one layer down. The retry is skipped if
another dialog opened meanwhile.

**Also here, because they are the same reader.** Toast text was set while the
live region was `hidden` and the region unhidden afterwards, which is not
reliably announced — VoiceOver says nothing. The regions now stay in the
document and the bubble inside them is what appears; errors moved to
`role="alert"`, because urgency is not a style. 12 px timestamps in
`neutral-400` were about 2.5:1 against a 4.5:1 floor, and read as decoration
because they were too faint to read. Checkboxes are exempt from the 44 px
touch-target rule, which was giving a `size-5` box a 20 by 44 target — the
range input was exempted for exactly this reason and checkboxes were missed.

**Cost, and a new thing to run.** `app/tests/` gains a headless harness and
two probes, because both of these defects were invisible to review and obvious
in a browser. It is manual: it needs a real Chrome, which the build images do
not carry, and pulling one in to run two probes is a poor trade. A PR that
touches the interface should say whether it was run — which is worse than CI
and much better than the nothing that was there, and saying so plainly beats a
check that quietly never runs.
