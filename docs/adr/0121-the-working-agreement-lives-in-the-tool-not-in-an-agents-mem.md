# 121. The working agreement lives in the tool, not in an agent's memory

**Status:** Accepted.

**Chosen:** the agreement moves into `config.toml` as an `[agent]` table, read
and written by `clispeak prefs`. The skill reads it rather than remembering
it, two of its rules are enforced by the tool, and a `UserPromptSubmit` hook
puts it back in an agent's context on every prompt.

**Why.** The agreement is what the skill is for and it was the part that did
not survive. It was stored by telling each agent to remember it — which is
per-harness, so Claude Code, Codex and Antigravity each remembered separately
and diverged; invisible to the person it describes, who could not read or
correct it; and lost the moment a write was forgotten, at the exact moment an
agent is being corrected and is least likely to do bookkeeping.

**Reading is idempotent and remembering is not.** An agent that forgets to
read simply reads again. An agent that forgets to write has lost the
preference for good. That asymmetry is the whole argument.

**Lists rather than prose.** Preferences do not change by rewording; they gain
a rule or lose one. A prose `speak_when` would mean rewriting the sentence on
every correction and quietly dropping clauses nobody noticed — the same lossy
write, moved somewhere new. As a list, "stop telling me about builds" is a
removal.

**Three layers, and they degrade well.** The hook survives compaction and is
Claude Code only. Enforcement needs no context at all and works anywhere.
Tool output is the floor. Bypass one and the others hold.

**Only two rules are enforced, deliberately.** `output = terminal` and the
identification opener. The obvious third is `never_speak`, and it does not
survive contact: what people want kept quiet are *categories*, a substring
match cannot check a category, and making it enforceable would mean listing
the literal secrets in a plaintext file in order to avoid saying them aloud.
Even literal terms misfire — `credentials` would refuse "the credentials test
passed". A tool that refuses correct messages gets `--raw` and then gets
abandoned.

**Nobody edits this by hand.** Patrick's requirement, and it removed a
question rather than adding one: the "switching modes has to be fast" problem
disappears when nobody types the command. "Heading out, but tell me if the
deploy fails" is two changes in one sentence, which no keyboard shortcut could
have carried.

**What it costs.** Provenance on every rule, because if nobody opens the file
a wrong rule is invisible and permanent — so each records which agent added it
and when. And a read-back after every change, which is one line of friction
and the only way the user ever sees what was stored.

**Found by testing, not by reasoning: the file needed a lock.** Twenty
concurrent `prefs add` calls recorded fourteen rules and silently lost six.
The first fix was wrong in the way its own comment warned against — the lock
went around the write while the read stayed outside it, so two agents both
read the old list and the second overwrote the first. Reading inside the lock
takes it to twenty of twenty, four runs running.

**Where the hook lives, and why not in the skill.** A skill can declare hooks
in its frontmatter, which would be tidier — removing the skill would remove
the hook by construction. The exact frontmatter shape for a *command* hook is
not documented clearly enough to guess at when it is the mechanism the whole
design rests on, so it goes in `settings.json`, whose shape is documented, and
`clispeak forget` removes it explicitly. Worth revisiting once the frontmatter
form has been seen to work.
