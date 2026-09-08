# 53. Every gate is one command, and `main` never cancels its own run

**Status:** Accepted.

Two failures with one shape, found together. Issue #91.

**A cancelled run looks exactly like a passing one.** `cancel-in-progress` was
true for every ref, so three consecutive pushes to `main` landed with no
verdict at all — not a failing verdict, none — because each newer push killed
the run before it reported. `main` was red on clippy for two commits and
nothing said so. It is green again only because the fix landed by accident
inside an unrelated later commit, and it was found by a teammate whose own
pull request went red on a line they had never touched.

Cancelling on a branch is right and saves real money. On `main` it removes the
verdict from the branch everyone else starts from, so it is now
`cancel-in-progress: ${{ github.ref != 'refs/heads/main' }}`. The rule this
project opens with is "check the CI run, not your terminal", and that rule
needs there to be a run.

**The local gate was a chain assembled by hand.** The commit that introduced
the lint was made with `fmt && portability && test && commit` — clippy simply
was not in the list. That is the third variation this week of the same
mistake: a `&&` that short-circuited so an edit never ran while the previous
output still printed, a `;` where `&&` was meant so a failing gate did not
stop a push, and now an omission. Each time the gate itself was correct and
the wiring around it was not.

`cargo xtask check` runs all four, cheapest first, printing each step's name
before it runs and stopping at the first failure. Named steps because a gate
that passed and a gate that never ran otherwise look identical in a
scrollback — the same reasoning as the counts `portability` prints. It cannot
be mis-chained because there is nothing to chain.

**What this does not fix.** Nothing requires either of these before a merge:
`main` still has no branch protection and no required checks, which is #5 and
Patrick's to set. A gate nobody is obliged to run is a convention, and this
week is what a convention is worth.
