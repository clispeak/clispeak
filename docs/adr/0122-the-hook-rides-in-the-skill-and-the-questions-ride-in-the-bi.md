# 122. The hook rides in the skill, and the questions ride in the binary

**Status:** Accepted.

**Chosen:** the `UserPromptSubmit` hook is declared in `SKILL.md`'s own
frontmatter rather than written into the user's `settings.json`, and the setup
questions are printed by `clispeak prefs setup` rather than written into the
skill's prose.

**Why the hook moved.** The first version edited `~/.claude/settings.json`
behind a `--hook` flag, because the frontmatter shape for a *command* hook was
not something to guess at when it is the mechanism the whole design rests on.
Reading the documentation properly settled it: a skill may declare hooks, they
register when the skill is invoked, and they keep running for the rest of the
session.

That makes the frontmatter strictly better. Nothing of the user's is edited.
Installing the skill installs the hook, so there is no second step to forget.
And **removing the skill removes the hook by construction** — where a hook
written into somebody's settings file outlives the skill it came with and goes
on running a command they believed they had removed. 129 lines of settings
merging, tidying and removal went with it.

The cost is that the hook is registered when the skill is *invoked* rather
than at every session start. That is the right trade: no cost at all in
sessions where clispeak never comes up.

**Why the questions moved the other way.** They belong in the binary for the
same reason the agreement does. A skill installed months ago asks last year's
questions and does it confidently; questions that ship with the binary are
current by construction, and an agent with a stale skill still asks the right
ones. `prefs setup` also knows which have already been answered, so it can be
run again without starting from nothing.

**When they are asked, which is the part that changed twice.** The original
skill demanded a five-question interview before an agent knew enough to use
the tool — a form standing in front of every first message, which agents route
around by not using the tool. Removing it went too far the other way: asked to
"run the full setup", the answer was that there wasn't one. The moment that
works is the first time clispeak comes up in a conversation: already an
engagement, and once rather than per message.

**What `fallback-to` deliberately does not do.** Retrying an unreachable
device automatically was considered and refused. Whether an unreachable phone
is worth chasing to every other device is a judgement about *this* message,
not a property of the failure — a rule that broadcast to `all` whenever a
phone was off would be loudest exactly when nobody is there to hear it. The
tool records the preference; the agent decides.

**A one-directional gate, found while doing this.**
`crates/clispeak-cli/tests/skill.rs` fails when the skill names a command that
does not exist. It says nothing when the *tool* grows a command the skill
never mentions — and it had grown five. The drift it was built to catch has a
direction it cannot see.
