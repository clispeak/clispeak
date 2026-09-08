# 22. Windows refuses to start rather than starting silent

**Status:** Superseded — see the first line below.

**Superseded by #18.** `voicecastd` starts on Windows. Piper runs there now, so
the choice this decision was made between no longer exists.

**What was decided, and why it was right.** With no Windows engine at all, a
daemon that started would accept messages, report `queued`, and say nothing.
The sender could not tell that from working. Refusing to start was the honest
option, because silence that reports success is worse than a service that is
plainly absent.

**What changed.** Not the reasoning — the alternatives. Windows has an engine,
so refusing would now take a node off the network over an install that can be
fixed, and a sender learns no more from a daemon that is absent than from one
that explains itself. Where Piper is missing or will not start, the node runs
with `SilentEngine` carrying Piper's own reason.

**What that costs.** The reason string is now the only thing standing between a
Windows user and an unexplained silence, so it has to name something a person
can act on. "Install the Microsoft Visual C++ Redistributable" is actionable.
`0xC0000135` is not. A reason that degrades into a bare error code puts this
decision back where it started, without the honesty of refusing.

<details>
<summary>The original entry, kept because the reasoning still holds</summary>


**Decision.** `voicecastd` on a platform with no speech engine fails at
startup with a message naming the app as the node there, instead of falling
back to a silent engine.

**Why.** A node that accepts messages and says nothing reports `queued` to the
sender and then does nothing at all. The sender has no way to tell that from
working, which is the exact failure the whole design avoids elsewhere by
sending reasons back rather than swallowing them.

**Cost.** Anyone testing IPC on Windows has no daemon to talk to. That is the
right trade while no Windows engine exists: the app links `voicecast-core`
directly and is the node on every desktop anyway.

</details>
