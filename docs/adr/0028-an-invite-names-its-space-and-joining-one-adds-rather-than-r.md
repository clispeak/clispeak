# 28. An invite names its space, and joining one adds rather than replaces

**Status:** Accepted.

**Decision.** A ticket carries the space it invites into. `accept_join`
honours that rather than whichever space happens to be default when the
joiner arrives. `revoke` and `rotate` take a space too. And joining a space
*adds* it, except when the space it would displace holds only this device.

**Why the ticket carries it.** Deciding at the moment the joiner arrives
answers a different question from the one the person pressing the button was
answering. Change the default between showing a QR code and it being scanned,
and the device lands somewhere nobody chose.

**Why joining had to stop replacing.** `do_join` called `replace_current`,
with a comment saying joining means adopting a space's membership rather than
blending it. That was right when a device held exactly one space and became
wrong the moment it could hold several — a device in `home` that joined `work`
**lost `home`**. So multiple spaces could only be created locally and never
joined into, which is most of what anyone would want them for. Mine, from
decision 20, and not revisited when the assumption under it changed.

**The exception is the space every node founds for itself.** A fresh device
holds one space containing only itself. Joining should displace *that* —
otherwise a first pairing leaves an abandoned empty space beside the real one.
`Spaces::current_is_unshared` draws that line, and is tested for the case that
matters: a space with somebody else in it is never displaced.

**A joined space needs a local name**, because labels are local and the space
carries none. Numbered — `space-2` — rather than guessed from the inviter,
since a guess that collides is worse than a placeholder, and the response says
how to rename it.

**Unknown space names are an error, not a fallback to the default.** Acting on
the wrong space is precisely the failure a per-space button would otherwise
produce, and it is the reason the interface refused to offer those buttons
until this landed.
