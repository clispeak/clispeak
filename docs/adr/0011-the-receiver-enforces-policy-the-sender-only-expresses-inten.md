# 11. The receiver enforces policy; the sender only expresses intent

**Status:** Accepted.

**Decision.** Mute, quiet hours, and volume are receiver-side and cannot be
overridden remotely. `--priority high` may interrupt a message, but whether it
breaks through quiet hours is a per-device toggle, off by default. Whatever the
receiver decided is reported back honestly.

**Why.** The alternative fails immediately: the first time an agent decides
everything is urgent, "urgent" stops meaning anything, and a device you muted
starts talking. Silent discards are equally bad — the agent must be able to
learn a message was suppressed.

**Related.** This is what makes `no_engine` a first-class status rather than a
silent failure — a Linux receiver with no voice model downloaded reports it.
