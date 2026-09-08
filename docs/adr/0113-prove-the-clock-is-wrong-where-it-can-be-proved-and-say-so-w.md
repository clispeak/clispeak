# 113. Prove the clock is wrong where it can be proved, and say so where it cannot

**Status:** Accepted.

An invite is valid for five minutes and expiry is judged against the clock of
the device *reading* it. Patrick met that on a fresh Windows VM whose clock was
wrong: an invite made seconds earlier reported itself expired, and the message
said "that invite has expired, ask for a new one" — which sends the reader to
mint another that will fail identically, for as long as they are willing to
keep trying (#200).

**"Expired" was never a property of the ticket.** It is a statement about two
clocks, reported as a fact about the paper. That is the shape `CLAUDE.md`
opens with: a tool answering honestly about something adjacent to the question
asked.

**One direction is provable and it is now proved.** A ticket is minted with
exactly `TTL_SECS` of life, so no honest ticket can carry more than that. More
than that means the reading clock is behind the minting one, by at least the
difference. That is a fact, and it is stated as one — with the amount.

**The other direction cannot be proved and is not guessed at.** A clock running
ahead and a genuinely stale invite produce identical bytes; nothing in the
ticket separates them. So that message hands over the evidence instead of a
verdict: how long ago it expired, and what this device believes the time is.
Someone whose clock is hours out reads the second half and diagnoses it in a
second. Guessing on the unprovable half would be the same mistake wearing a
new coat.

**What was left alone, having looked.** #200 claimed the inviting device
repeats this fault over the wire when it refuses a join. It does not: that
device minted the ticket with its own clock and reads it with the same one, so
there is no disagreement to misreport and "expired" there is simply true. The
issue was wrong and the code was right.
