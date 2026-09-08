# 8. Native TTS engines out of the gate

**Status:** Accepted.

**Decision.** Ship with free, built-in, no-API-key synthesis. Pluggable
`SpeechEngine` so cloud voices can be added later.

**Why.** Nothing to sign up for, nothing to pay for, works offline.

**Cost.** Linux has no usable native engine, so Piper is bundled there —
~20–60MB per voice model. Voice quality varies across platforms.
