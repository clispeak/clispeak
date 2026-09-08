# 92. A test that signs and verifies agrees with itself

**Status:** Accepted.

Decision 83 recorded the domain separator inside `Member::signed_payload`
being renamed with the project, voiding every membership signature on every
device with nothing said. Decision 85 made the silence audible. **Neither
added a test that would have stopped it**, and the obvious one does not.

Two tests came out of trying, and the pair is the point.

**The one that reads like the answer and is not.** `peer_tests::
a_join_produces_a_membership_that_verifies` drives a real join against a
node's own `handle_peer` — ticket, `invite`, reply — and then runs the
*joiner's* `Roster::adopt` over the record the host handed out. That closes a
genuine gap: the accept path had no test above the unit level (#80), and this
one fails if the host records a member under a different name than the joiner
advertised, or leaves one side a member and the other not.

It does **not** catch the rename. Tried, with the constant put back to
`voicecast-join-v1`: it still passes, because both ends call the same function
and a test that signs and verifies with one constant agrees with itself
whatever that constant says. The break only exists *between two builds*, and a
single process contains one build.

**The one that does.** `signed_payload_tests::
the_signed_payload_is_a_fixed_shape_that_past_signatures_depend_on` asserts
the exact string. Spelled out, not built with `format!` — a test that derives
its expectation the way the code does cannot disagree with it. Compared as
text rather than bytes, because a failure has to be readable and two 38-byte
arrays are not, and carrying a message that says what a change here means.

**The decision, stated generally: a value that past artefacts were computed
over is pinned by a written down copy, never by a round trip.** Round-tripping
tests the code against itself. That is the whole family CLAUDE.md's table is
about — domain separators, ALPNs, keyring service names, config directory
names, wire tags — and the test for every one of them has this shape.

The doc comment on `signed_payload` now says the same thing in the place
someone renaming it will actually be looking, and the test says explicitly:
**if this fails, do not update the expected value.** It is not describing the
code, it is describing signatures already sitting on devices that will never
be rebuilt.

**Costs.** One assertion that has to be edited deliberately if the format ever
does change on purpose, which is the intended cost. And the join test carries
a paragraph explaining what it does not do — worth more than the line it
saves, because a test named after a property it does not have is worse than no
test.

**Adding the second node test also exposed a race the first one hid.**
`node_for` sets the process-wide config directory, which is a `OnceLock`: the
first caller wins and every later one silently gets the first one's directory
back. Two node tests running at once therefore shared an identity, a roster
and a pending invite, and one deleted the directory while the other was using
it. It passed for as long as there was exactly one such test, and then failed
in the suite while passing on its own — the least useful shape a failure has.
The tests are serialised on a mutex, `node_for` uses one directory per process
rather than one per test, and both facts are written where the next person
adding a node test will read them. Same shape as the rest: a setter that is
ignored does not say so.
