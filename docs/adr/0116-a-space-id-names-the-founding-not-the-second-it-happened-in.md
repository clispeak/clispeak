# 116. A space id names the founding, not the second it happened in

**Status:** Accepted.

**Chosen:** `Roster::found` sets the space id to the derived
`founder:joined_at` plus a random nonce, so every founding produces a
distinct id. `derived_id` itself is unchanged.

**Why.** The derived id said *who* founded a space and *when*, which is the
right idea, and "when" was `joined_at` — unix seconds. Two spaces founded by
one device inside one second therefore derived the same id, and
`Spaces::insert` is a map write: the second silently replaced the first,
taking every device paired into it.

Three ordinary ways to reach it. `clispeak space new work` followed by
`clispeak space new home`. A `rotate` on a space founded moments earlier.
And any scripted or agent-driven setup at all — which is the case that hits
it every time rather than occasionally, and this project exists to be driven
by an agent.

**The tell was in the suite, again.** `two_spaces_founded_by_one_device_are
_still_two_spaces` reached into the second roster and added a second to every
`joined_at`, under a comment reading "force a different founding moment, which
is what distinguishes them". The bug was written down as a given by whoever
worked around it. That is the second time in one day: decision 115 was two
tests sleeping 1100ms for the same reason. Both are the shape this codebase
keeps meeting — not a failure, an absence, wearing something that reads like
care.

**Why a nonce rather than finer time.** Milliseconds would shrink the window
rather than close it, and would still be a clock. The id does not need to be
*when*; it needs to be *which*. Random says that directly, and needs no
persistence — a counter would have to survive restarts to stay unique, and a
space id has to stay unique for the life of the space rather than the life of
a process.

**What it costs.** Nothing that can be measured. The id is an opaque string
and nothing parses it; a joiner is told the space's id by the host and adopts
it rather than deriving one, so both ends agree as before. Spaces already on
disk keep the id they were saved with — only new foundings differ — so an
upgrade keeps every existing space and every pairing in it. `derived_id`
remains the fallback for a roster that arrives with no id at all.

**Found by** the first tests ever written against `resolve`, which #80 had
recorded as the most intricate function in the crate with no tests at all. The
fixture needed a device in two spaces; building one destroyed the first space,
and the resolver was blamed before the roster was.
