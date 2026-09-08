# 14. Reject bad text; don't silently rewrite it

**Status:** Accepted.

**Decision.** The CLI validates before sending. Markdown and bare URLs are
**errors** carrying the offending spans, a concrete suggested rewrite, and a
named escape hatch. Emoji and stray whitespace are handled silently. Exit code
`6`, distinct from usage errors. `--strip` converts instead of rejecting;
`--raw` skips validation.

**Why, and why this is better than the original proposal.** The first design
was to normalize silently — strip markdown, summarize URLs, speak the result.
The caller is an agent, though, and **an agent can act on an error message**:
read it, fix the text, retry. That makes validation a working feedback channel
rather than an obstacle, which it would be for a human piping a file.

It also produces better speech. Text an agent *wrote to be spoken* beats
markdown mechanically stripped after the fact — "Updated 3 files" is a better
sentence than anything a rewriter extracts from `Updated **3 files**`.

**The line.** Error when there is an obvious correction the agent could make;
handle it quietly when there is not. Markdown has an obvious plain-prose
alternative. Emoji does not — an error on 🎉 would be a puzzle with no answer.

**Cost.** Breaks `cat CHANGELOG.md | voicecast`, since a changelog is markdown. That
now requires `--strip`, which makes the conversion explicit rather than
silent — an acceptable trade, and arguably clearer.

**Knock-on benefit.** With markdown rejected at the door, chunking is no
longer a rewriting problem. It reduces to finding safe sentence boundaries,
which is a much smaller and more testable job.
