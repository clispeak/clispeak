# 108. One machine, one address book, or the node refuses to start

**Status:** Accepted.

Patrick's call on #198, 6 September 2026, after watching his own laptop report
four real devices and then two strangers within a minute — same device id,
same machine.

**The condition.** The device identity lives in the system keyring under
`("clispeak", "device-identity")`, with nothing in that name derived from
where the configuration lives. So every config directory carrying
`identity.in-keyring` names *the same secret*, and is therefore the same
device — while keeping its own roster, history and mute settings. Two such
directories on one machine, and whichever node binds the socket first decides
which address book the CLI and every peer sees.

**A device like that does not look broken.** It looks like a device that has
forgotten who it knows, which is a much more expensive thing to chase — this
project has already spent a day on a space dissolution whose signature was
exactly that (#166). No claim is made that #166 was this; it was traced to a
roster merge and that explanation stands. The point is that a second way to
produce the same symptom was sitting in the ordinary act of having a packaged
build and a native build at once.

**And it is not new.** `app/src-tauri/src/lib.rs` still carries the comment
from the last time it happened: overriding the config directory on desktop
"gave a single device two rosters, two histories and two mute settings,
sharing only the identity that lives in the keyring". That fix removed one
cause. The condition it created stayed reachable, and a user reaches it by
installing the Flatpak and a native package, in either order.

**So the node refuses to start**, names both directories, and says the fix is
to delete the one that is not wanted or to set `CLISPEAK_CONFIG_DIR`. Refusing
is the decision, not a side effect of one: the alternative is starting
successfully and being subtly wrong, which is the failure mode this whole
repository is organised against.

**Three things it deliberately does not do.**

It does not choose. Only the person knows which roster is the real one, and a
node that picked the newer or the larger would be right most of the time,
which is the worst available property.

It does not fire on a directory holding `identity.key` instead of the keyring
marker. That is a different device with its own secret and no claim on ours.

And it does not fire when `CLISPEAK_CONFIG_DIR` is set. A directory the caller
named is a decision already taken — including the two-node setup `CLAUDE.md`
documents for testing, which shares a keyring identity by design and would
otherwise be refused by a check written to protect it.

**What it costs.** Any machine that already has both directories stops
starting until someone clears one, and that includes this one — the laptop
this is written on carries a stale directory from before the rename, holding
two devices that no longer exist. That is the check working, on the first
machine it meets, and it is still a cost paid by whoever updates.

**What it does not fix.** The duplication itself. One config directory per
machine — the sandbox using the host's — is the answer that makes this
impossible rather than reported, and it is a data migration with real state on
the wrong side of it. #198 keeps that option open and this closes the hole in
the meantime.
