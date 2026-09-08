# 37. A peer that names a space is answered about that space, or not at all

**Status:** Accepted.

Three separate bugs turned out to be the same sentence written wrong. A
message carries the space it is about; the receiver has to decide which of
its rosters that means; and in each case the code answered with a *different*
space rather than admitting it did not hold the one asked for.

**`space_for` guessed.** Its fallback exists for peers old enough not to name
a space at all, but it fired for a named space we no longer held, answering
with any other space that peer was in. A phone that ran `space leave work`
therefore merged every work device into its `home` roster on the laptop's next
presence check, and those devices could then speak to it — the work message
arriving on the family tablet that having spaces at all is meant to prevent.
The fallback is now reached only when nothing was named.

**`accept_join` fell back to the default.** A ticket names a space by id. If
that space was gone, the joiner was admitted to whatever was default *now*,
which after `rotate` is the space the rotation just built. A QR photographed
before the panic button was pressed stayed scannable for the rest of its five
minutes and let its holder into the replacement. Rotating, leaving and
dropping a space now cancel an open invite, and a ticket for a space we do not
hold is refused with a reason rather than redirected.

**A refused sync revoked from the wrong space.** `JoinRefused` dropped the
peer from `space_of(peer)`, which answers with the default first, rather than
from the space the sync was about. A peer leaving a second space was removed
from the one it was still a member of.

**Why refusing beats guessing.** Every one of these was a fallback written to
be helpful, and each was silent: the wrong answer looked exactly like the
right one, and the failure surfaced days later as a device that could speak
where it should not. The cost of refusing is a person who has to show a new
invite. The cost of guessing is a message in the wrong room.

**Related: the joiner now adopts the id the inviter names.** A roster derives
its id from the founder's own entry, which stops working once the founder
leaves — each side derives from a different member and holds one space under
two ids, after which every message between them names a space the other does
not have, and takes the fallback above. `adopt_id` existed for this and had no
callers. Issues #50 and #51.
