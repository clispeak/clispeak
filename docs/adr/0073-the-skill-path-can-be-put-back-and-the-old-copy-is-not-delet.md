# 73. The skill path can be put back, and the old copy is not deleted

**Status:** Accepted.

**Chosen:** a "Use the default location" link beside the skill path, shown
only when the path is not the default. It forgets the recorded location; it
does not remove the file already written to it.

Patrick pointed the skill install at a directory under his Desktop, and there
was no way back to Claude Code's own path short of retyping it from memory.
The interface let a choice be made and offered no way to unmake it.

**It had to forget the record, not refill the field.** `skill_status` reads
`skill-destination` from the config directory, so a button that only wrote the
default into the input would have been undone by the next five-second poll —
a control that appears to work, then silently reverts, for a reason nothing
on screen explains. Falsified by making the reset leave the record in place:
three of the probe's checks fail, including the field's value after one poll.

**The old copy stays where it was.** It is the user's file, written where they
asked. Deleting it is a different act from changing where the next install
goes, and this button did not offer to do it. But a skill left somewhere that
will never be kept in step is exactly the silence this project keeps paying
for, so the reset reports the path it stopped tracking rather than saying
nothing about it.

**Hidden when the path already is the default**, because a reset that resets
nothing teaches the reader that the control does nothing. The interface
compares against the *field* rather than the reported path, so it appears as
soon as someone types a different one rather than a poll later.

**`default_path` is nullable, and that was a review catch.** The first version
wrote `skill_default()?`, which on a machine with no home directory would have
returned no status at all — hiding the entire skill panel, on exactly the
machine where someone with a recorded path most needs to read it. Reachable
only where `BaseDirs::new()` is `None`, and arguably a system that cannot
install a skill anywhere; but the failure mode is *a section that disappears
rather than explaining itself*, which is the bug decision 68 was written about
and which this app shipped once already. Verified by returning a null default
in the harness: the section stays and the button hides, which is the answer
wanted in both halves.

**Costs.** One more Tauri command and one more thing `skill_status` carries.
The reset leaves the skill uninstalled at the default until Install is
pressed, which is honest — the badge says `absent` and the button is directly
below it — but it is two steps where a combined "reset and install" would be
one. Two steps was chosen because the combined version writes a file without
being asked to.

**Why it surfaced at all.** The custom path is what made macOS ask the app for
Desktop access, which read as a hung launch for an hour — the app was waiting
on a permission dialog that is not the keychain's, so a check for
`SecurityAgent` reported no prompt and was looking in the wrong place. The
feature request came out of the diagnosis, from Patrick, who could see the
screen.
