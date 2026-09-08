# 31. Joining is a read and then a decision, not a single button

**Status:** Accepted.

**Decision.** Joining a space happens in two steps: `preview` reads the ticket
and reports what it would join; `join` acts on it. The app's Spaces screen does
the same in one dialog, and the button that opens it sits above the space list
rather than below it. Every action that acts on one space moved behind a single
**Manage** button on that space's card, in a dialog titled with the space.

**Why joining cannot be offered per space.** The destination is written into
the ticket when the invite is minted. The joining device does not choose it and
cannot — so a Join button sitting on the `work` card would promise a
destination it has no ability to deliver, and would say `work` while the code
said `home`. The honest alternative is not to let the joiner choose but to let
them *read*: `Ticket::parse` is local, contacts nobody and does not spend the
single-use token, so there is no cost to showing the answer before committing.
The errors a bad code produces — expired, truncated, not an invite — surface at
the preview instead of after the fact.

**Why the ticket now carries a name as well as an id.** `Ticket.space` is a
space id, `<founder>:<joined_at>`, which is exactly right for selecting a
roster and useless to show a person. The label rides alongside, and
`JoinAccepted` carries the inviter's live name too — the two differ when a
space is renamed between minting an invite and it being scanned, and the live
one wins.

**The bug this closes (#36).** Joining `work` produced a space called
`space-2`. The name travelled on both hops and `do_join` read neither, naming
every joined space with a counter. Since spaces are addressed by label, that
made `work/laptop` a guess on the device that had just joined — and it would
have made the new preview a lie, promising `work` and delivering `space-2`.

**Why the joiner still gets the last word on the name.** A label is local: it
is how one device writes `work/laptop`, nothing is sent when it changes, and a
name already in use here has to give way because two spaces called `work`
would make that selector mean two things. The inviter's name is the default
because agreeing is the common case, not because it is authoritative.

**Why one Manage button.** The card footer had grown to five controls, each
having to name its target — "Leave work", "Replace work" — to be unambiguous,
and they wrapped to three rows on a phone. With the name in a dialog title the
buttons need no qualifier. Adding a device stayed on the card, being the one
thing here that is done often.

**Cost.** Joining is two taps rather than one, and a scanned invite no longer
joins on its own — it opens the same confirmation, which is a step for someone
who scanned deliberately and a rescue for someone who did not. Managing a space
is one tap deeper than it was.
