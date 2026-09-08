# 86. A device is called one thing, in every space, on every start

**Status:** Accepted.

Issue #147, seen on a phone: **Settings → This device** said `Phone` while
**Spaces → main** said `Android phone`, seconds apart, on a fresh identity.
The laptop agreed with Settings. So the roster entry a device held *for
itself* carried a different name from the one it advertised to everyone else,
and the device — the authority on its own name — was the one that had it
wrong.

The cause was never reproduced. Two defects in the startup reconciliation
were, and either produces exactly this:

- It ran on `current_mut()` — **the default space only**. A device in two
  spaces was corrected in one of them. This is the same "every space, not
  just the default" bug that `rename` carries a comment about having already
  fixed once, made again in the other place that touches the same field.
- It was **skipped entirely on a migrating start**. The branch that saves a
  freshly migrated `spaces.cbor` returned before the reconciliation, so a
  device coming through the legacy-roster migration kept whatever name that
  roster held while advertising whatever `device_name()` said.

**The decision: `adopt_own_name` runs on every start, over every space, and
says when it had to correct one.** `device_name()` is what the last rename
wrote down, what every join request carries and what `--to` matches, so a
roster entry disagreeing with it is stale by definition. The direction is not
new — `rename` has always pushed the file's answer into the rosters — it is
now applied where a device gets its second chance to notice.

The announcement matters as much as the correction. This was found by a
screenshot and could not be reproduced from one; a line naming both strings
turns the next occurrence into evidence.

**Costs.** A device whose name file is lost while its rosters survive would
adopt the fallback name rather than keeping the good one. Both live in the
same directory and move together through the migration allowlist, so that is
a directory half-deleted — and a device that cannot read its own name is
already wrong on the settings screen and in every join request it sends.
