# 90. The history is written off the path of a message being spoken

**Status:** Accepted.

Every `SpeakBegin` and every outcome wrote the whole history file — up to 200
entries of 100,000 characters — under a `std::sync::Mutex`, on a tokio worker.
`write_private` calls `sync_all` before it renames, deliberately, because a
rename that lands before its contents leaves an intact name over an empty
file. So **delivering a message waited on an fsync**, on a phone, with latency
proportional to how much history the device had accumulated (#78).

Not a correctness bug on its own, which is why it sat: everything worked, and
worked more slowly the longer you had used it.

**The decision: serialise under the lock, write outside it, and coalesce.**
`History::to_bytes` is the serialising half of `save`, so a caller can take a
consistent snapshot while it holds the lock and hand the bytes to a `Saver`
that owns the file. The lock is released before anything touches the disk.

**A thread and a one-slot mailbox, not a queue.** The slot holds the *latest*
snapshot rather than each one in turn, so five outcomes in a row cost one
write: a snapshot that has not been written yet is not a lost update, it is a
state the newer one already contains. That is what makes this cheap rather
than merely asynchronous.

**A thread rather than `spawn_blocking`**, because the outcome callback is
handed to the queue and can run with no tokio runtime around it. A history
write that panicked for want of a reactor would be a worse bug than the one
being fixed. And if the thread cannot be started at all, `put` writes
synchronously rather than dropping the snapshot — slow is the failure this
type exists to avoid, and a history that silently stops recording is a worse
one.

`Node::close` flushes, because that is the one place waiting for a write is
right: nothing is racing it, and the alternative is losing the last outcome
recorded. `Drop` joins the thread for the same reason.

**Costs.** A window in which the file is behind the in-memory history. It is
bounded by one write and closed by `close`, and the failure it replaces —
losing the newest entry on a hard kill — was already possible with the
synchronous write, which could be interrupted between the entry and the
rename. `Saver` owns the path so there is exactly one writer; nothing else in
the node may write that file, and the field it used to reach it with is gone.
