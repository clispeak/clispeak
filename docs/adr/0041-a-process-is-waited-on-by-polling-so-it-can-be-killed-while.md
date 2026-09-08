# 41. A process is waited on by polling, so it can be killed while we wait

**Status:** Accepted.

`stop` is the whole of an interrupt. The trait says "immediately, mid-sentence",
and the queue's urgent messages, `skip`, `clear`, `pause` and `stop_message` all
reduce to it. It did not work in any engine that spawns a process: `finish` held
the `current` mutex across the wait, `stop` began by taking that mutex, so it
parked until playback had finished and then found nothing to kill. Issue #58.

Both things an engine must do to a child need `&mut Child`, from different
threads, at the same time. `std` has no portable way to kill a process by
handle from elsewhere, and inventing one means platform code in three places.

**So the wait polls.** `try_wait` under the lock for an instant, release, sleep
10 ms, repeat. `stop` takes the lock in one of those gaps and kills. The cost is
a hundred wakeups a second against a process busy synthesising speech, which is
not measurable; the bound on `stop` is the poll interval, which is far below
what anyone hears as a delay. Measured on a Mac: a `stop` that took **13.1
seconds** now takes **26 ms**, and the message reports `cancelled` where it used
to report `spoken`.

**A deliberate kill is not a failure.** Killing a process makes it exit
non-zero, so checking exit status (#59, decision 40) would have turned every interrupt
into "the player crashed". `Running` records that it was killed before it kills,
and reports success for that exit. The queue already checked its own `cut`
before the engine's error for this reason; this makes the engine honest on its
own rather than relying on every caller to know.

**Reaping.** `kill` without a `wait` left one zombie per interrupt for the life
of the daemon, and interrupting is something a person does repeatedly.
