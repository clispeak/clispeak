# 23. The agent skill lives in this repo, and a test keeps it honest

**Status:** Accepted.

**Decision.** `skills/voicecast/SKILL.md` is versioned here, and
`crates/voicecast-cli/tests/skill.rs` fails if it names a command or flag the
binary does not have.

**Why in-repo.** The skill is installed into agents and describes this CLI. A
copy kept anywhere else drifts the moment a flag is added, and "we will keep
it updated" is not a mechanism.

**Why a test rather than a note.** A skill that has drifted is worse than no
skill: it states with confidence that a flag exists, and the agent obeys it.
Generating the skill from clap would remove drift entirely, but most of its
value is judgement — when speaking is worth doing, what a `muted` reply means
— and judgement cannot be generated. Checking the generated-checkable part is
the useful half of that trade.

**What the skill carries that `--help` cannot.** That every message should
name the user and the agent, because a person with several agents cannot
otherwise tell whose voice is in their pocket. That exit 4 with `muted` is a
decision to respect, not a failure to retry or route around. That the failure
worth avoiding is not a missed message but being muted, after which every
later message is missed too.

**Consequences.** The skill establishes a working agreement with the user once
and instructs the agent to record it, so the preferences survive the session
that set them. Changing the CLI now means changing the skill in the same
commit, which is the point.
