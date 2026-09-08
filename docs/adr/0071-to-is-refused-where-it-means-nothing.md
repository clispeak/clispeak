# 71. `--to` is refused where it means nothing

**Status:** Accepted.

**Chosen:** an explicit `--to` on a command that cannot honour it is a usage
error, not a flag to ignore. Speaking, `stop`, `skip`, `pause` and `resume`
take it; everything else refuses it and says so. A `default_target` from the
config is untouched — only a flag the caller typed trips this.

**What #121 was.** `--to` is a global option, so clap accepted it on every
subcommand including the ones that can never act on another device:

```console
$ voicecast --to Bravo mute
muted:   yes
$ echo $?
0
```

`Alpha` was muted. `Bravo` was not. The user named one device and a different
one went silent, with exit 0 and nothing said about it. Six commands behaved
this way — `mute`, `unmute`, `status`, `queue`, `history`, `groups` — and two
of them change state.

**The design it collides with was right.** `mute`'s own help says one device
cannot mute another, because the device making the noise decides whether noise
is welcome. That is a good decision. The bug is that a flag contradicting it
was accepted in silence rather than refused.

**Hard error rather than a warning**, on Patrick's call. A flag that names one
device and changes another is worth failing over, and it is cheaper to refuse
now than once somebody's scripts depend on the silence.

**Written out as an exhaustive match, with no catch-all arm.** A command added
later will not compile until somebody says which side of the line it is on.
The bug was a subcommand quietly inheriting a flag nobody had considered it
having, so the fix should make that inheritance impossible rather than merely
wrong.

**The skill was teaching it as a feature.** `SKILL.md` said `stop`, `skip`,
`pause`, `resume` *and `queue`* all take `--to`. Four do. An agent following
that would read this device's queue believing it was the phone's — the
documentation asserting the bug, which is the third place a limitation is owed
and the one that reaches an agent directly. Fixed here, with `docs/cli.md`.

**Costs.** A behaviour change: a script passing `--to` harmlessly to `status`
today starts failing. That is the point, and the error names the command and
prints the line to run instead. `--to here` is refused too, even though it
would be a no-op — one rule is easier to hold than one rule with an exception.

Found by running two nodes on one laptop and driving the CLI at them, as #116
was. Neither was visible by reading.
