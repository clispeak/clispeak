# 32. A device's name is asked of the system, not read from a Linux file

**Status:** Accepted.

`device_name` fell back to the hostname, and the hostname was
`/etc/hostname` — a file that exists on Linux and on none of the other four
targets. macOS and Windows have a perfectly good name and no such file, so
both fell through to the placeholder and *every* device was called
"this device", the remote ones included. Issue #38.

That is worse than an ugly name. Devices are addressed by name, so
`--to "this device"` matched several members and took the first, silently:
two nodes on this machine, and the report named the device that spoke as
"this device" — which was true of both. The one thing the report exists to
say was the thing it could not say.

`gethostname` is asked instead, through a crate rather than a
`#[cfg(target_os)]`, because `voicecast-core` may not have one and this is
precisely the kind of divergence a portable call is for. `rustix` and
`windows-link` were already in the tree.

**Why `.local` is stripped and other domains are not.** Every Mac answers
`Patricks-Mac-mini.local`; the mDNS suffix is identical on all of them and so
distinguishes nothing, which is this name's only job. Trimming *every* dotted
part is the tempting generalisation and is wrong: `host.corp` and `host.home`
are two machines, and collapsing both to `host` would recreate the ambiguity
this decision removes.

**Why `localhost` is refused.** A phone or a container answers with it, and it
is the same on every device that does — a placeholder that looks deliberate.
Refusing it keeps the per-platform fallback ("Android phone", "iPhone") that
was already there for the case where the system has no useful answer.

**Cost.** A device that was already named keeps its name: the stored name and
`VOICECAST_NAME` still win, and only a device that never had one changes. One
new dependency. The names two devices in different domains get can still
collide, and addressing still resolves a duplicate silently rather than
refusing — that is issue #39, not this change.
