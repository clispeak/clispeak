# 25. The device that speaks decides how long a caller waits

**Status:** Accepted.

**Decision.** `--wait` is bounded by an estimate the *receiving* device makes
from the text, its own engine rate and everything already queued — not by a
constant, and not by the sender. An explicit `--timeout` overrides it.

**Why not a constant.** A constant is wrong at exactly one length. The flat
120 seconds held until someone sent 569 words, at which point the device spoke
all of it and the caller was told it had not finished.

**Why not a bigger constant.** This is the part worth keeping. On an M4,
Piper's `en_US-lessac-medium` produces **147.62 seconds of audio** for those
569 words, but takes **179.5 seconds end to end** — about 1.22x, because
chunks are synthesised one at a time and each pays its own start-up. Raising
the limit to 180, which is what measuring the audio alone would suggest, would
have failed again on the same message on the same machine by half a second.

**Why the receiver.** The sender knows the words and nothing else that
matters. The engine, its rate, and the queue in front of the message all live
on the device that will speak it — and for a message arriving from a peer, the
sender could not have known them at all.

**The assumed rate is 100 wpm**, well under any engine here — Piper measures
197–231, espeak-ng defaults to 175. The estimate bounds how long a caller
*may* wait, not how long it does: waiting ends when speaking does, so
over-estimating is free and under-estimating is the entire defect.

**Three shapes, all measured rather than argued:**

| | Measured |
|---|---|
| A long message | 569 words: 147.62s audio, 179.5s end to end |
| A message queued behind one | 14 words reported `speaking` at 120.4s, having not yet been spoken at all |
| A message being spoken | counted for nothing: 40 chunks playing, `depth() == 0`, `pending_words() == 0` |

The third was found in review, because a doc comment claimed the message being
spoken was counted whole and the code walked only the queues — from which the
thread removes a job in order to speak it.

**Consequences.** The estimate is made where the speaking happens, so it only
takes effect once the *receiving* device carries this. Until then `--wait`
against an older device still cuts off at 120 whatever the sender runs. That
belongs in release notes.

Fixing this also surfaced that `--timeout` never crossed the wire at all: it
was IPC-only, so `--timeout 600 --to Phone` did nothing whatsoever. It travels
on `SpeakBegin` now, optional and defaulted, so an older peer sending nothing
reads as expressing no preference rather than asking for zero.
