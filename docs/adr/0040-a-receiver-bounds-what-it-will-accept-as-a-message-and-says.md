# 40. A receiver bounds what it will accept as a message, and says why it refused

**Status:** Accepted.

The sender chunks text through `voicecast-text` before streaming it, and the
receiver took that as a guarantee. It is a courtesy: what arrives is whatever
the peer chose to call a chunk. Nothing checked the length of one, nothing
counted them, and nothing ended the loop but a `SpeakEnd` the peer might never
send. A member could stream 8 MB frames until the phone killed the app, or
send one long chunk that filled Piper's output pipe and held the speech thread
for the life of the process. Issue #53.

**Oversized chunks are re-chunked, an oversized message is refused.** The two
differ because they are different questions. A chunk that is too long is a
peer that chunks differently — older, or written by someone else — and
splitting it costs nothing and keeps them working. A *message* past what this
device will say in one go is not a formatting difference, and quietly speaking
the first hour of it would be worse than refusing.

**100,000 characters, about two hours of speech.** Far past anything anyone
sends. The number is not the point; having one is. It is a limit on what this
device will synthesise in a breath, not on who may talk to it.

**`Report` gained a reason.** `Rejected` already meant "you are not in this
space" and now also means "that message was too long" — two things an agent
has to tell apart, since one is fixed by pairing and the other by sending less.
The field is `#[serde(default)] Option<String>`, so a peer on an older build
simply sends none, which is the compatible-addition pattern this wire format
was chosen for. It surfaces in `voicecast --wait` and in the app's toast,
both of which already showed `detail` for local failures and had nothing to
show for remote ones.

**What this does not fix.** The receiver still cannot interrupt a chunk once
the engine has it, because `stop` waits on the same lock playback holds — that
is #58, and it is why the bound above matters more than it otherwise would.
