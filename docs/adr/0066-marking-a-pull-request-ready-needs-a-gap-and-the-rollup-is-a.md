# 66. Marking a pull request ready needs a gap, and the rollup is a history

**Status:** Accepted.

**Chosen:** do not mark a pull request ready in the same breath as a push, and
before merging count five *named* targets in the check rollup rather than
looking for the absence of a warning. Written down because the rule reads as
fussiness until you know why, and because it is currently enforced by a person
remembering it.

**What happened.** #106 was pushed and marked ready about a second apart. The
push's own run had already evaluated the pull request as a draft — so decision
63's `if:` skipped the matrix — and the `ready_for_review` event never became a
run at all. The result was a pull request reading as ready, with one check
present and passing, and four of five targets never built. Toggling draft and
back with thirty seconds between produced a run immediately, so the trigger is
fine and the race is narrow and real.

It was very nearly merged. That is the whole reason this is written down.

**The rollup is a history, not a status.** `matrix.target: SKIPPED` is the
draft run's result and it stays there permanently, beside the four real names,
on a pull request that has been through the full five. The draft run's Linux
job stays too, so seven rows for five jobs is normal — two of them the same
name.

**So the check is the set of distinct names, not a count.** This decision
first said "count five names", which is wrong for a reason found by it going
wrong: five green *rows* is satisfied by the duplicated gate job plus three
targets while the fourth is still running. It read green on #114 with Android
in flight, against a rule written the same afternoon, by the person who wrote
it. "No SKIPPED row" rejects every correct pull request; "five green rows"
accepts an unfinished one; only naming the four targets and the gate job
answers the question that is being asked.

**This is decision 63's sharp edge and it belongs to that decision, not to
whoever trips on it.** Making the expensive jobs conditional bought most of a
month's CI allowance back and introduced a state where a pull request can look
finished and not be. Every other silent failure this project has met had a
gate that could have caught it; this one has none. The five-target rule — the
line `CLAUDE.md` opens with — is enforced here by a person counting.

**The real fix is not this decision.** Required status checks naming the five
would make a pull request with four that never ran unmergeable regardless of
what anyone read, which is "check the CI run, not your terminal" applied to
merging rather than to claiming. That is #5 and it is Patrick's to make. Until
then this is a convention, and conventions are what this file exists to
record the cost of.

**Costs.** A pause between pushing and marking ready, which is nothing, and a
rule that fails silently when forgotten, which is not. Noted rather than
solved.
