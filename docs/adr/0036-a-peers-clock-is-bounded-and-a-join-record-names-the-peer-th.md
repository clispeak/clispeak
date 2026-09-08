# 36. A peer's clock is bounded, and a join record names the peer that sent it

**Status:** Accepted.

The roster is a set of signed records merged from whoever we are paired with,
and every rule in it is a comparison between two numbers the *sender* chose:
a revocation beats a join record by being newer, a rename beats a rename by
being newer, and a rejoin beats a revocation the same way. Nothing bounded
those numbers. One member could sign its own record with `joined_at: u64::MAX`
and win all three for ever — unrevokable, and revocable of everyone else by
the matching tombstone. The signature was real; only the number was a lie, so
every check we had said yes. Issue #48.

**Records dated ahead are refused; tombstones are clamped.** The two are
treated differently because one is signed and the other is not. `joined_at`
is inside the signed payload, so it cannot be corrected without breaking the
signature — the record can only be taken or left, and a record dated further
ahead than clock drift explains is left. A tombstone carries no signature at
all, so it can be clamped, and dropping it instead would let a device with a
fast clock stop a revocation spreading.

Tombstones clamp to *now*, not to the skew ceiling. A revocation cannot
honestly have happened later than the moment we heard of it, and a ceiling
five minutes ahead would still beat every rejoin signed in the next five
minutes — which is exactly the window someone re-pairing a device is standing
in. `renamed_at` sits outside the signature too, so an impossible stamp is
read as no rename at all, leaving the label already held.

**Five minutes.** Larger than the drift of a device that is merely wrong;
anything that has reached an NTP server is within seconds. Small enough that
winning by it buys nothing, since the forged record ages into the past while
the space carries on. The cost is real and worth naming: a device whose clock
is more than five minutes fast cannot be admitted, and the failure is a
refused record rather than a message about a clock. That is the wrong error to
show, and it is the price of not carrying a second, unsigned notion of time.

**A join request no longer says who is joining.** `accept_join` signed the
endpoint id the *message* carried, never comparing it against the QUIC peer
that sent it. They match every time a real client asks, and where they differ
is the whole attack: a ticket holder enrolling a third key it does not hold,
so revoking the device in front of you removes nothing. The connection is the
authority; a mismatch is refused rather than silently corrected, because a
client that disagrees with itself is worth a reason. Issue #52.

**Ids must parse as keys.** An id that is not a key can never be dialled, but
it could be stored, synced onward and printed — and printing it panicked,
since ids are shortened to sixteen *bytes* for display and a multi-byte
character straddling that boundary is not a char boundary. That ran inline on
the node's IPC task, so being wrong once cost the whole node. Both halves are
fixed: `verify` refuses the id, and shortening counts characters.
