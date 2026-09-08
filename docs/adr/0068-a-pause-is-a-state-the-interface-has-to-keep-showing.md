# 68. A pause is a state the interface has to keep showing

**Status:** Accepted.

**Chosen:** a held message is reported as what this device is playing, `stop`
ends a pause as well as the queue, and the playback panel stays on screen
whenever speech is held — with or without a message to name.

**What #109 was.** Pause, and the app went mute for good. Every message after
it was accepted, acknowledged with a toast, and never spoken. Only restarting
the app recovered it.

Three things had to line up, and all three were reasonable on their own:

1. Pausing moves the message out of `speaking` and into `resume`, because
   nothing is coming out of the speaker. Literally true.
2. `snapshot().speaking` was therefore `None`, so `now_playing` returned no
   `msg_id`.
3. The interface hides the playback panel when nothing is playing — and the
   panel holds the Resume button.

So the control that undoes a pause was hidden by the pause. Nothing else in
the interface mentions the state, and `paused` is a mode that silently
swallows every later message.

**"Nothing is being spoken" and "nothing is playing" are different claims,
and the queue was making the wrong one.** A held message is what the Resume
button will continue, which is what a person means by "what is playing". So
`speaking` reports it, and `pending` stops counting it, since a message cannot
be waiting behind itself.

**Stop had to become a way out rather than a deeper way in.** `clear()`
emptied every queue and left `paused` set, so the control a person reaches for
when a pause has gone wrong left the device just as mute with nothing left to
resume. It now ends the pause too.

**And the panel stays visible while paused even with nothing held**, because
a pause can arrive from the CLI with an empty queue. Then there is no message
to name, and the only other evidence is that the device has gone quiet.

**Skip had to learn about pauses, because this fix is what made it
reachable.** A cut interrupts whatever is in flight, and while paused nothing
is: the message is held and the thread is asleep, so the cut sat unread until
the next message picked one up and discarded it. Skip did nothing. Nobody had
noticed because the button was hidden behind the same bug — which is the
shape worth naming. Fixing a control's visibility exposes every control beside
it, and those had never been exercised in that state either.

**Falsified rather than assumed, in both halves.** Two queue tests fail
against the old code. A new browser probe fails every check against the old
`now_playing` contract — the panel disappears on pause and the button still
reads "Pause" — which is Patrick's report reproduced in a harness.

**The test double was wrong in a way that hid this shape.** `FakeEngine::stop`
latched unconditionally, so a stop with nothing playing made the *next*
message fail with "stopped". No real engine does that — Android's reads its
stop generation fresh on every `speak`, so a stop before one is simply not
there. A double stricter than the thing it stands in for reports failures the
product does not have, and this one appeared while chasing a failure the
product does. It now only latches while something is in flight.

**Costs.** `speaking` now means "the message this device owns right now",
which is a slightly wider claim than the field's name. The CLI prints `held`
rather than `speaking` for it, because `paused` on one line and `speaking` on
the next is a contradiction and an agent reading it would conclude audio was
coming out. The browser probe needs a real Chrome and is not run by CI, so it
is a command someone runs and a pull request that says whether they did.
