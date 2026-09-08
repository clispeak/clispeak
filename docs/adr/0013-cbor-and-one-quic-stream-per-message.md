# 13. CBOR, and one QUIC stream per message

**Status:** Accepted.

**Decision.** CBOR encoding. One bidirectional QUIC stream per message, plus a
long-lived control stream per peer. Protocol version negotiated in `Hello`.

**Why CBOR over postcard.** Postcard is smaller and faster, but it is not
self-describing. This project has severe version skew by construction — a
desktop updates on a package manager run, an iPhone updates when App Store
review finishes and the user opens the app. Old peers must be able to skip
fields they have never heard of rather than silently misparsing them.
Envelope size is irrelevant anyway; the text payload dominates.

**Why stream-per-message.** It resolves three problems for free rather than
designing each: cancellation becomes a stream reset (no cancel/chunk race),
priority isolation becomes independent flow control (a long document cannot
head-of-line block an urgent notification), and backpressure becomes QUIC's
flow control (a receiver reading at synthesis speed stalls the sender, so
piping a huge file cannot exhaust a phone's memory).

**Cost.** It commits us deeply to QUIC. Any non-QUIC fallback transport would
mean reimplementing cancellation, prioritization, and backpressure by hand.
Accepted — iroh is the transport, and iroh is QUIC.
