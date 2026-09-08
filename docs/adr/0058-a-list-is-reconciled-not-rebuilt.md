# 58. A list is reconciled, not rebuilt

**Status:** Accepted.

Both lists called `replaceChildren` on every five-second poll. That threw away
three things nobody had finished with: an expanded message collapsed mid-read,
a focused Remove or Play button dropped focus to `<body>` so a keyboard user
lost their place entirely, and a Play button showing "…" was reset while its
request was still in flight. Issue #74.

All three live on the node itself — a class, the focus ring, a disabled
attribute — so they survive exactly as long as the node does. `syncRows`
matches rows to data by key, patches the ones that survive, and builds only
what is new.

**Nodes are moved only when the order actually changed.** A move is a remove
and an insert as far as the DOM is concerned, and that blurs a focused
element, so reordering everything to prepend one new message would have
defeated the point. Prepending now touches only the new node.

**Keeping a node means keeping its handlers current.** A card's Manage button
closed over the `space` and the space *count* of the moment it was built, so a
card that survived a poll would have opened Manage with a stale `is_default`
and, once a second space existed, without the actions that only appear when
there is more than one. The handlers are rebound on every poll. This is the
cost of reconciliation and the part that is easy to miss: the bug it
introduces is invisible until the facts change under a card nobody rebuilt.

**Unheard-only asks deeper.** The filter runs in the interface, and it ran
after a 50-entry request — so an unheard message that had fallen outside the
newest 50 was hidden from the one view whose purpose is finding it. Filtering
now asks for more than the node retains, so it sees everything there is.
Server-side filtering is the better shape and wants a wire field and a CLI
flag to be coherent; this fixes the defect completely with respect to what is
kept, which is all that exists to find.
