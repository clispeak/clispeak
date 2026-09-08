# 42. A process that ran and failed is not a process that spoke

**Status:** Accepted.

`wait()` returning `Ok` means the process was successfully waited for. It was
being read as "the audio was heard". A player exiting 1 — `paplay` on a box with
no PulseAudio session, `aplay` with no ALSA device — reported `spoken`, which is
the one thing "report what happened" forbids. Issue #59.

Both statuses are now checked, the synthesiser first: Piper failing while the
player reads a truncated stream and exits 0 is a real shape, and the
synthesiser's failure is the one worth reporting.

**`EngineError` gains `Failed`.** A missing engine and a failing one need
opposite responses — one is installed, the other is diagnosed — and collapsing
them told someone whose audio device had gone to download a voice model. The
variant carries the command, the exit code, and a bounded tail of stderr.

**Why stderr is drained on a thread.** Reading it after the process exits
deadlocks as soon as the output passes the pipe buffer: the child blocks
writing, so it never exits, so nothing ever reads. A hang is worse than the lost
message being fixed. The tail rather than the head, capped at 1 KB, because the
last thing a process says before dying is the part worth having.

**What this does not reach.** `queue.rs` maps any engine error to
`Status::NoEngine` and discards its text, so the reason assembled here still
does not arrive at the sender. That is a separate defect of the same shape this
project keeps producing — the reason exists and something between it and the
reader drops it — and it is filed rather than fixed here, because the mapping
sits in the crate being reworked under the security cluster.
