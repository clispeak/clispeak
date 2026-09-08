# 119. `CLAUDE.md` gets audited, and the first audit found three stale claims

**Status:** Accepted.

**Chosen:** `CLAUDE.md` is maintained like the rest of the docs — corrected in
the same change that makes it wrong, and audited when something suggests it
has drifted. Patrick granted this on 7 September 2026, on the condition that
every edit is reported.

**Why now.** The merge rule in that file counts named checks, and #226 added a
sixth. A rule one name out of date passes a pull request whose newest check
never ran, so leaving it stale was the dangerous option rather than the
cautious one. Having edited it once, a pass over the rest was owed.

**What the pass found.** Three, and the third is the one worth reading.

1. **`cargo xtask check` had gained a gate the comment never mentioned.**
   `versions` has run since decision 112 and the line still listed six steps.
   Harmless on its own; it is how a file starts being read as approximately
   true.

2. **"keep them in step by hand".** The CLI's copies of `clispeak-core` have
   been checked by `tests/drift.rs` since decision 117. Left alone, the file
   would have gone on telling the next person to rely on remembering, when
   something now tells them. The startup figure moved with it, from a
   remembered "~3ms" to a measured 2.0ms over 200 runs.

3. **The file contradicted itself.** "Linux and Android get real runtime
   testing; the rest are build-verified" sat four hundred lines below a table
   this project added on 6 September saying Windows had been *launched* — and
   Windows had by then been paired, spoken through and installed from a
   release artefact. Two claims about the same fact, in one file, disagreeing,
   for a day.

**Why that third one matters more than its size.** The file's entire subject
is a thing that answers honestly about something adjacent to the question
asked. A document that says two different things is the same failure in the
medium meant to prevent it: whichever half you read is confident, and nothing
marks the other.

**What was left alone.** `docs/decisions.md` still says "~3ms" in four places.
Those are records of what was believed when each decision was made, the file
is append-only, and correcting history would destroy the only thing it is for.

**What it costs.** A pass over one file, occasionally. Against a merge rule
that silently stops matching the checks it counts, that is not a close call.
