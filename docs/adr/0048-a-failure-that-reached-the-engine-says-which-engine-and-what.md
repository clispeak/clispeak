# 48. A failure that reached the engine says which engine, and what it said

**Status:** Accepted.

Decision 42 made the engines distinguish "there is no engine" from "an engine
ran and failed", and assemble a real reason for the second: the command, how
it ended, and a bounded tail of its stderr. None of it reached the sender. The
queue mapped every `Err` to `Status::NoEngine` and dropped the string, one
layer above where the work had been done. Issue #86.

**Why that mattered more than a lost string.** The two failures want opposite
responses. A missing engine is installed; a failing one is diagnosed. A
receiver with a working Piper and no audio session reported "no engine", so an
agent reading the report went and installed a speech engine that was already
there and already fine — while the sentence that would have fixed it in a
minute had been produced and discarded.

**The queue's outcome is now a status and a reason.** `Ended` replaces the
bare `Status` on the finish channel, the `OnFinish` callback and
`speak_job`'s return. `speak_here` already returned a `detail` and had nothing
to put in it; `TargetResult` has carried one all along, and `PeerMessage::Report`
gained one in decision 40, so this was the last gap in a path that was
otherwise complete end to end.

**An engine that ran and failed reports `Unreachable`, not `NoEngine`.**
Neither is a perfect fit, and `Unreachable` is the better lie: it says the
device could not be made to speak without claiming anything about what is
installed, and the reason underneath says the rest. A status meaning "the
output device failed" would be more honest and would cost every receiver a
variant it must be taught; that trade is worth revisiting if a second case for
it appears.

**`EngineError` is now `Clone`**, so a test can hand the same failure to an
engine twice. Two tests, one per variant, because the whole point is that they
are no longer the same thing.
