# 123. The hook's frontmatter carries no `args`, and a test says so

**Status:** Accepted.

**Chosen:** the `UserPromptSubmit` hook in `SKILL.md` declares `command:
clispeak prefs --brief` and **no `args` key**, and
`crates/clispeak-cli/tests/skill.rs` fails if one appears.

**Why.** The first version had `args: []`, copied from a documented example
whose `command` was a path to a script taking no arguments — where an empty
args array is consistent. Generalised to a command *line* it is not: with an
explicit `args` present the runner treats `command` as a bare executable name
and does a literal `$PATH` lookup, so every prompt produced

```
exitCode: 1
stderr:   Executable not found in $PATH: "clispeak prefs --brief"
```

and the agreement never reached the model.

**How it hid.** From inside the conversation nothing looked wrong. The hook
fired, the transcript recorded the failure, and the agreement appeared to be
known — because it had been read by hand a few minutes earlier during setup.
The mechanism that exists to survive a compaction was dead, and the only
evidence was in a log nobody had reason to open. Patrick found it by reading
the transcript.

**The claim that was carried across a change it did not survive.** The hook
had been verified working — as a `settings.json` entry, whose form was
correct. When it moved into the frontmatter (decision 122) the verification
did not move with it, and "the hook fires" went on being said about a
different mechanism. Evidence is about the thing it was gathered from.

**What the test pins.** Not a full YAML parse: the one key whose presence
changes how the command is executed, plus that the command has arguments at
all — a hook with none would not need this guard. Falsified by putting
`args: []` back and watching it fail.

**What it cost to find.** Nothing, this time, because it was caught before
release. What it would have cost is the failure the skill file describes in
its own words: after a compaction the agreement is out of context with nothing
to put it back, silently.
