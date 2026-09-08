# 97. A dozing phone is not an absent one

**Status:** Accepted.

Decision 89 bounded the peer dial at twenty seconds and wrote down the cost:
*"A device on a network slow enough to need more than twenty seconds is now
called unreachable when it might have connected. Nothing measured has come
close."* Something measured has now come well past it, and the sentence was
wrong in a way worth naming: nothing had been measured *on the case that
matters*.

**Measured on 4 September 2026, against Patrick's actual phone**, by speaking
to it three times:

| | took |
|---|---|
| first message, phone idle for hours | **58.0s** |
| immediately after | 2.1s |
| again | 2.3s |

Twenty was chosen because M0 timed a relay-first connection across
carrier-grade NAT at about a second, and twenty looked like an order of
magnitude of headroom. It was headroom over the **warm** case. The cold case —
a phone that has been in a pocket, dozing, for hours — is nearly thirty times
slower, and it is not an edge case here. **It is the ordinary one.** The whole
premise of this project is reaching someone who is not at their machine, so
almost every message that matters is a first message to an idle phone.

At twenty seconds that first message would have been reported `unreachable` to
a phone that was about to answer. That is a worse failure than the one
decision 89 fixed, and it would have broken the thing the tool is for while
looking like a correct timeout.

**The decision: ninety seconds**, in the node and in the CLI's mirror of it.
Well past the one cold sample, still a bound, and the cost is that a device
genuinely switched off takes that long to be called unreachable — slow and
true, against ten seconds and false, which is what decision 89 replaced.

**How it was found is the part worth keeping.** Not by testing: by trying to
speak to Patrick and having it fail, on a build that predates the fix, with
the exact wrong message decision 89 was written to remove. The failure was
real, the diagnosis was one command (`clispeak status` answered instantly, so
the node was fine), and the measurement took three more.

**What is still not known** is *where* those 58 seconds went. `took_ms` covers
the whole delivery — dial, stream, speech, report — so it does not prove the
dial was the slow part, and the bound this changes is on the dial alone. It
could be that connecting took four seconds and a dozing Android took fifty-four
to actually speak. The number is therefore set from the total, which is the
conservative reading: if the dial is the fast half, ninety is merely generous;
if it is the slow half, ninety is necessary.

The next cold message on a build carrying decision 89 will say which, because
a dial that times out now reports `connecting to peer: no answer in 90s` and a
slow speech does not. One sample of one phone on one network, and it is
written down as that.
