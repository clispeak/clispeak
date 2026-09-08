# 64. Two gates that check the tree before the tree is checked

**Status:** Accepted.

**Chosen:** `cargo xtask check` runs two lexical gates before it runs cargo —
unresolved conflict markers in any tracked file (#103), and a multi-line
`run:` step that is not a literal block scalar (#102).

**Order is the point of the first one.** The bug was not that markers survive;
it is that a *green verdict is returned about a tree nobody has finished
merging*. `all gates passed` printed while `docs/decisions.md` held a live
marker, because the numbering check reads headings and markers do not disturb
the sequence, and fmt, clippy and test never open a markdown file. Every other
gate's answer is meaningless in that state, so this one runs first or it is
decoration.

**The second gate is about a form, not a symptom.** A `run:` written as a
plain scalar or as `>` folds its newlines into spaces, so a shell line
continuation survives as a literal backslash and the shell reads it as an
escaped space — the next path becomes an argument with a leading space naming
a file that does not exist. In #97 that left a properties file holding three
passwords on disk while the step exited zero, because `rm -f` is silent by
design.

The rule is `|`, not "a block scalar": `>` folds exactly as the plain form
does, checked against the same input rather than read off the spec, so a rule
phrased against the plain form would certify the trap. And the check matches
the *form* rather than hunting for backslashes on purpose. The file is correct
YAML and correct shell separately; the meaning is lost in the handover. A
backslash detector would be a check about the symptom, and would look like an
improvement.

**The exemption list is empty, which was not the plan.** Both gates were
expected to need one: this file describes the markers, and the checker's own
source spells them out. Neither does, because matching only at the start of a
line already separates a marker from prose about one.

That is worth more than the saved lines. `docs/decisions.md` is the file that
conflicts on every rebase — all three of today's conflicts were in it — so an
exemption arrived at by reasoning would have excused the single file most
likely to carry a real marker, and the gate would have looked correct while
being blind exactly where it was needed. Verified by putting a real conflict
in that file and watching it be caught. The list is kept, empty, so a document
that one day must open a line with a marker can be named where it is read.

**Falsified before being trusted**, each against the real thing rather than a
sketch: #97's removal step pasted back verbatim in the plain form, the same
step rewritten as `>`, markers in a document and in a frontend source file at
once, and a sentence quoting the markers inline — which correctly does not
fire.

**The workflow gate shipped a false positive and was caught by review, not
by itself.** `split_run_key` measured a step's indent before the list dash, so
`- run:` was reported two columns to the left of where it is, and every
sibling key of a *one-line* run step read as "deeper than the key" — the test
for a folding plain scalar. Any step written dash-first with `name:` or `if:`
after it would have been flagged for a fold that is not there. No workflow
here is written that way, so the gate passed at 26 steps while being wrong,
which is the same latency as `cfg(unix)` sitting in a portable crate while the
gate printed "3 crates clean". Fixed, then re-probed against six shapes rather
than the four the first version was checked against.

**The conflict gate reported everything three times, and only when it
fired.** During a conflict the index holds three entries for a conflicted path
— stages 1, 2 and 3 — and `git ls-files` prints all three, so the file was
read and scanned once per stage. Nine lines where three belong, on the exact
screen where someone is working out what to fix. It cannot happen outside a
conflict, because a merged index has one entry per path, so the bug was
invisible in every test either of us would write and certain in the only case
the gate exists for. `tracked_files` now sorts and deduplicates, rather than
using `--deduplicate`, which is a newer git flag than this has to run on.

That is the sharpest instance of the day's shape: the thing only misbehaved
when it fired, so no amount of testing it in the ordinary state would have
found it.

**And the answer is not always another gate.** Writing these, clippy caught
`indent.len().max(0)` on an unsigned integer in the checker itself —
unconditionally true, silently. Neither new gate would have seen it; `-D
warnings` did. That is #103's argument pointing the other way: the value is
not in adding checks, it is in the ones already there continuing to run and
continuing to be read.

**Costs.** Two more gates to run, both milliseconds. The workflow check is
lexical, so a `run:` key inside a folded block of some other key's value would
confuse it — no workflow does that, and a YAML dependency in `xtask` to rule
it out would be a larger cost than the case is worth. And these catch a
shape, not an outcome: a `run: |` step whose shell is wrong is still wrong,
and a file with no markers can still be half-merged.
