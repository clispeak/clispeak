# 12. Multiple spaces per device, one keypair

**Status:** Accepted.

**Decision.** A device may belong to several fully separate spaces at once,
holding one roster per space but keeping a single identity. Messages carry
their space so the receiver applies per-space policy. Leaving is local removal
plus a gossiped self-revocation.

**Why multi-membership rather than move-between.** The real case isn't
relocating a device, it's overlap: a personal phone should hear both work and
home while the work laptop hears only work. A model that only supports moving
can't express that, and "leave and rejoin" would be a daily chore.

**Why one keypair.** Per-space keypairs isolate better but need multiple iroh
endpoints per device and multiply the key management. For a mesh of one
person's own devices the isolation buys nothing — the same NodeId appearing in
both of your own spaces is invisible to anyone.

**Settled: spaces are never shared with another person.** That removes the
only reason per-space keypairs were ever on the table, and with it a class of
design work — no roles, no permissions, no approval workflows, no per-device
ACLs. Membership is the only authorization concept in the system, and revoking
retires your own device rather than ejecting a person.

**No cross-space "all".** `--to all` is scoped to a single space; reaching
several requires naming them. The failure this prevents — a work message
arriving on the family tablet — is the entire reason someone made separate
spaces.
