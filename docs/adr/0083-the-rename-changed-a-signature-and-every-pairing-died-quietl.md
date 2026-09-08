# 83. The rename changed a signature, and every pairing died quietly

**Status:** Accepted.

Decision 82 renamed 844 occurrences of the project name. One of them was not a
name:

```rust
format!("clispeak-join-v1:{endpoint_id}:{invited_by}:{joined_at}")
```

That string is the domain separator **inside the bytes every membership
signature is computed over**. Changing it did not rename anything; it
invalidated every signature ever produced, because the verifier now hashes
different bytes than the signer did.

**What it looked like from outside.** The phone joined and reported *"1 of 1
device"*. The laptop logged `no longer shares that space with us; removed` and
revoked the phone. Wiping the phone gave a fresh identity and the identical
failure. Two clean identities, same result — which pointed at the code, and
the code was fine: two fresh local nodes joined each other perfectly, because
both of their rosters had been signed under the *new* constant.

The reason it presented as a join bug rather than a signature bug is
`Roster::adopt`:

```rust
for member in members {
    if verify(&member).is_ok() {
        roster.members.insert(member.endpoint_id.clone(), member);
    }
}
```

A member that fails verification is **dropped without a word**. So the laptop
sent three members, the phone silently discarded all three — including the
laptop's own entry, signed years of commits ago under the old string — and
ended up with a roster containing only itself. The laptop went on listing
Laptop and iOS locally the whole time, because local listing filters on
tombstones and never re-verifies. Both sides were internally consistent and
told the truth about what they held. Neither could say why the other's
members had gone, because neither had noticed them going.

**The decision: take the break.** `rotate` re-founds the space, which re-signs
the local member under the current constant, and everything pairs again. The
alternative — restoring `voicecast-join-v1` for ever, or verifying against
both — was rejected. The protocol identifier had already been broken by the
same rename, so every device needed rebuilding regardless; a constant naming
a dead project is a permanent puzzle for every future reader; and a
dual-verify path is permanent complexity for a transition exactly one
installation will ever make.

**Costs.** Every existing pairing is void. Three devices re-paired. Had this
shipped, it would have silently unpaired every install with no message saying
so, and the only recourse would have been a re-pair nobody was told to do.

**The lesson, which is this repository's oldest one in a new place.** The
table at the top of `CLAUDE.md` is about things that are not
platform-*shaped* and so are not caught by a gate looking for platform shapes.
This is the same failure with "name" in place of "platform": a rename tool
looks for names, and a cryptographic constant that happens to spell the
project name is not one. It is data that past artefacts were computed over.
**Grep the rename for strings that are hashed, signed, stored, or sent** —
domain separators, ALPNs, keyring service names, config directory names,
wire tags. Both of this rename's real bugs were in that set, and nothing
else was.

And `adopt` deserves the other half of the blame. An empty result and a
rejected result are not the same thing, but they are spelled the same way
here. It should count what it dropped and say so — filed as #145.
