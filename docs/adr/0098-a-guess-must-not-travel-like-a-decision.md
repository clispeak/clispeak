# 98. A guess must not travel like a decision

**Status:** Accepted.

Four devices went from a working space to mutual eviction in about thirty
seconds, during ordinary pairing, with nothing crashing and no error anywhere.
The founder ended up revoked by every other device and revoking two of them
back (#166).

```
06:55:32   the Mac is revoked
06:56:17   the Mac rejoins — 45s later
06:57:03   iOS joins, announcing the Mac's hostname as its name
06:57:11   the laptop revokes the Mac
06:57:12   the Mac revokes the laptop
06:57:32   the laptop revokes the phone
```

Every device was individually self-consistent the whole time. What they
stopped agreeing about was who was a member.

**Two defects, and only the second one scales.**

### A rejoin left the revocation in place

`admit` refuses a record older than the tombstone against it, and inserts one
that is newer — and never removed the tombstone either way. So a device that
was revoked and re-invited was **in `members` and in `revoked` at the same
time**, and `merge` copied that contradiction to every peer it synced with.

Membership was then decided by comparing two dates on every read, which works
until a clock, a clamp, or a merge from a device that missed the rejoin moves
one of them. **The decision: a record that beats a revocation clears it.**
Same rule on the merge path, where a tombstone that no surviving member
outlived is spent and is dropped.

A tombstone against a device that is *not* a member stays. That one is still
doing a job: refusing a replayed record.

### A refused sync was answered with a revoke

This is the one that took the space apart:

```rust
PeerMessage::JoinRefused { .. } => { … roster.revoke(&peer); }
```

The comment above it read *"safe by construction: a peer can only ever make us
forget itself, which it could already do by leaving."*

**True of one message, false of the system.** A revoke mints a tombstone,
tombstones travel through `merge`, and a tombstone applies on every device
that receives one. So one refused sync did not make *us* forget a peer — it
removed that peer from the entire space. When two devices refused each other
inside the same minute, the space unravelled.

**The decision: `forget` for a guess, `revoke` for a decision.** `Roster::forget`
drops a member here and mints nothing, so it cannot travel. The self-heal it
was written for still works and works better: a device that genuinely left
already announces it, because `leave` sends a roster carrying its own
tombstone — a decision, meant to propagate. This path exists only for the case
where that announcement went missing, and the next sync with any other member
now supplies the real tombstone. What it no longer does is treat one refusal
as a conclusion every device should adopt.

And the correction is cheap in the other direction: if the peer *is* still a
member, the next sync with anybody puts it back. **A forget that the next sync
undoes is the definition of the thing being a guess.**

**Costs.** A device that left while every other member was offline stays
listed here for longer — until some member that heard the farewell syncs with
us. That is a stale row in a listing. What it replaces is a stale row's worth
of doubt taking the founder out of its own space.

**What made it hard to see, and is not fixed here.** `clispeak devices` hides
revoked entries, so every device looked healthy right up to the moment members
started vanishing; the state that explained it was the state the tool filters
out. It was found by reading `spaces.cbor` directly on three machines. Both
remaining entries were also called the same thing, because **a simulated iOS
device takes the hostname of the Mac it runs on** — `device_name()` reaches the
hostname before it ever reaches the `iPhone` default. Both of those are filed
separately; either alone would have cost an hour.
