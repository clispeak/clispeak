# 89. The node bounds its own dial, so the CLI stops blaming it

**Status:** Superseded by [0097](0097-a-dozing-phone-is-not-an-absent-one.md).

```console
$ clispeak say --to iOS "hello"
error: the node accepted the request and never answered
It may be wedged. Quit the clispeak app and open it again.
```

The local node was fine and answering `status` immediately. The peer was
simply switched off. So the error named the wrong node and told the reader to
restart a healthy app (#151), found on the Mac while re-verifying iOS under
the new bundle identifier — the identifier change had left the phone needing a
re-pair, which produced a genuinely unreachable device to point at.

**Two bounds that were never compared.** `speak` delivers before it replies,
whether or not the caller is waiting; delivery dials the peer; and nothing
here bounded that dial, so iroh's own timeout applied at about thirty seconds.
The CLI gave an ordinary request ten. It therefore gave up twenty seconds
before the node had an answer, and printed the one diagnosis it had for a node
that does not reply.

`patience()`'s doc comment already stated the rule this breaks: *"the node
applies the same bound and should be the one to time out — it knows why, and
can say queued or speaking instead of leaving the caller to guess."* That
mirroring existed for `--wait` and for nothing else, and **the bound an
ordinary speak would have had to mirror did not exist to be mirrored.** It was
a default inside a dependency.

**The decision: name the bound, put it in the node, and mirror it.**
`Transport::connect` is wrapped in `PEER_CONNECT`, twenty seconds, and
`clispeak-cli` waits `PEER_CONNECT + MARGIN` for every request that can reach
another device — `say`, and also `stop`, `skip`, `pause` and `resume`, which
dial a peer the same way and had the same ten seconds.

Twenty because M0 measured a relay-first connection across carrier-grade NAT
completing in about a second, so it is an order of magnitude past the working
case and short enough that an agent is not left holding a request while a
device that is switched off is dialled.

**The constant is duplicated by hand in the CLI**, beside a comment naming the
one in `clispeak-core`, exactly as the socket name and the frame format
already are. `clispeak-cli` depends on `clispeak-proto` and `clispeak-text`
and nothing else, which is what keeps its startup at ~3ms, and that is worth
more than importing a `Duration`. The direction is what matters and the test
asserts it as an inequality rather than an equality: the CLI must outlive the
node's bound, so the node is the one that times out.

**And the message stops asserting what it cannot know.** It now says how long
it waited, and that a device being slow to answer is not what this is —
because the node reports that itself.

**Costs.** A `say` to a device that is switched off now takes about twenty
seconds to come back `unreachable` instead of ten seconds to come back with a
false diagnosis. That is slower and correct, and `docs/cli.md` says so. A
genuinely non-blocking send — reply first, deliver after — is a different
design and a different decision; this one only makes the existing answer true.

A device on a network slow enough to need more than twenty seconds is now
called unreachable when it might have connected. Nothing measured has come
close, and the number is one constant in two files when that changes.
