# 45. State is written privately and all at once, through one function

**Status:** Accepted.

Six files hold this node's state and five were written with a plain
`std::fs::write`, which gets two things wrong at once. Issue #56.

**Permissions.** A plain write creates 0644 in a directory created 0755. Only
`identity.key` was written owner-only. On a machine whose home is readable by
others, any local user could read every message this device had spoken, and,
during the five minutes an invite is open, read the token out of `invite.json`
and pair themselves. The directory is now owner-only too — not for the
contents, which are private on their own, but for the listing: an open invite
is a file that exists for five minutes and then does not, which tells anyone
watching exactly when to try.

**Atomicity.** A plain write truncates and then fills, so an interruption
leaves a file that is neither the old contents nor the new. A truncated
`spaces.cbor` makes `Node::new` fail, so the node will not start and the only
way out is deleting the file — which deletes every pairing the device has. A
truncated `policy.json` is worse for being quieter: it reads as "nothing
configured", so a muted device un-mutes itself, which is the failure a test in
that module was written to prevent by a different route. Android kills apps
abruptly, so this is not hypothetical.

Everything now writes a temporary beside the target, flushes it to the disk,
and renames over it. Beside, because rename is atomic only within one
filesystem and `$TMPDIR` is often another. Flushed before the rename, because
otherwise the name can land before the contents do and a power cut leaves an
intact filename over an empty file — exactly what the rename was meant to
prevent.

**`cfg(unix)`, and what that says about the gate.** Setting a mode has no
portable spelling, so `store.rs` carries `cfg(unix)` in a crate that is
supposed to hold no platform conditionals. `cargo xtask portability` does not
catch it: it looks for `cfg(target_os)` and `cfg(target_family)`, and this is
neither. That is a real gap and it is filed separately rather than papered
over here, because the gate's whole value is that it says "3 crates clean" and
means it.
