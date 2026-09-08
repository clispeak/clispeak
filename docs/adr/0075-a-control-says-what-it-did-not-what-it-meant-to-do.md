# 75. A control says what it did, not what it meant to do

**Status:** Accepted.

**Chosen:** `stop` and `skip` report `Dropped` when they ended nothing, and
`pause` and `resume` carry the truth in the `detail` field. No new `Status`
variant, so nothing on the wire changes.

**What #116 was.** Against an empty queue, every playback control claimed
success at something that never happened — measured on a live node rather than
read off the source:

| command | reported | truth |
|---|---|---|
| `skip` | `cancelled` | nothing was cancelled |
| `stop` | `cancelled` | nothing was cancelled |
| `pause` | `queued` | nothing was queued |
| `resume` | `speaking` | nothing is speaking |

`apply_control` returned a fixed status per control: the *intent*, not the
outcome. The project's own convention is that a receiver "returns a status and
a reason, never silence and **never a lie**", and its premise is that an agent
drives the CLI — so an agent that sends `stop` and reads `cancelled` records
work it did not cause.

**The reasoning was already in the same function, applied to one arm.**
`stop --id` distinguishes `Cancelled` from `Dropped` "rather than reporting a
cancellation that never was". This is that sentence applied to the other four.
The queue now returns what each control found — `clear` a count, `skip`,
`pause` and `unpause` a bool — rather than the caller assuming.

**`pause` and `resume` are only half-fixed, deliberately.** `Status` has no
word meaning "the control was applied": every variant describes a *message*.
The honest fix is a new variant, and an unknown enum variant fails the whole
CBOR decode, so adding one breaks any older peer — the constraint behind
`#[serde(default)]` on every field added so far. That is a wire decision and
Patrick's to make, so the status stays the nearest available word and the
detail says what actually happened. `resume` on an idle device still reports
`speaking` in JSON, which is the residue.

**Costs.** A caller keying on `cancelled` to mean "the command worked" now
sees `dropped` when there was nothing to do — a behaviour change, and the
point. And `Speaker::clear`, `skip`, `pause` and `unpause` return values that
most callers ignore, which is the price of the node being able to tell the
difference.

Found by running two nodes on one laptop and driving the CLI at them, as #121
was.
