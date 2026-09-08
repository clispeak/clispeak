# 15. Lazy roster sync, eager revoke, no gossip layer

**Status:** Accepted.

**Decision.** Roster digest in `Hello`; exchange and CRDT-merge on mismatch.
Revocations are actively pushed to every known member. No `iroh-gossip`.

**Why the asymmetry.** Additions are already handled by the signed join record
— a device vouches for itself on arrival, so sync is an optimization, not a
correctness requirement. Revocations are the opposite: a revoked device still
holds a valid signature, so a member that never receives the tombstone keeps
admitting it forever. The urgent path is deletion, not addition, which inverts
what a naive design would prioritize.

**Why no gossip layer.** At 2–10 devices, gossip is machinery without a job.
Direct exchange over the already-open control stream has no topic membership to
manage and fails comprehensibly. Revisit only if device counts grow well past
full mesh.
