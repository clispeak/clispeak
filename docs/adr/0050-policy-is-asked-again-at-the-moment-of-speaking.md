# 50. Policy is asked again at the moment of speaking

**Status:** Accepted.

A message was checked against policy once, when it was accepted, and a queue
takes time to drain. So a message accepted at 21:59 from behind a long
document was spoken at 22:10, inside quiet hours, because nothing asked again.
Issue #77.

**Quiet hours are about when noise happens, not about when a message
arrives.** That is the plain reading of the setting, and the alternative —
honouring the policy in force at acceptance — means the length of a queue
decides whether a device wakes somebody. The `Speaker` now takes a `MaySpeak`
gate and asks it before each message.

**Before each message, not each chunk.** Cutting a sentence in half at exactly
ten o'clock would be worse than finishing it, and the granularity that matters
is "did this device make a noise it was told not to".

**Refused rather than held.** A message the gate stops is dropped with the
status the policy gives it, which is what already happens to a message that
arrives *during* quiet hours: recorded as unheard, and reported to a sender
that waited. Holding it instead would mean a queue that silently grows all
night and empties at seven in the morning, which is a different product.

**The queue-depth rule is not re-applied.** It exists to drop a low-priority
message that would arrive too late to matter, and it has already been asked
once, at submit. Asking again with this message about to be spoken rather than
waiting behind anything would be a different question wearing the same name,
so the gate passes a depth of zero.

`Job` gained the space it arrived in, because per-space policy needs it and
nothing in the queue knew it. A replay passes `None`: that is this device
speaking its own history, not the space's message arriving again.

**Still open, deliberately.** `set_mute` stops the engine and leaves the
queue, so unmuting later speaks whatever was waiting. That is arguably right —
`stop` is the command for clearing — and it is now the only half of #77 that
is undocumented rather than fixed.
