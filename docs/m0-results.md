# M0 results — iroh on cellular

**Verdict: pass.** The transport assumption holds, and on two counts it is
stronger than the design assumed. No architectural change required.

Run 2026-09-01. Linux laptop (home wifi) ↔ Pixel 10a. Single carrier, LTE.
Phone binary cross-compiled with NDK r29 and run headless via `adb shell` —
no app, deliberately, so this measures the *transport* and not Android's app
lifecycle. That is M6's job.

## Results

| # | Condition | Direct | Median RTT | Notes |
|---|---|---|---|---|
| 1 | Phone on wifi | ~100% | ~16ms | Baseline. |
| 2 | **Phone on cellular (CGNAT)** | **91%** | **155ms** | Hole-punched through CGNAT. |
| 3 | Cellular, relay forced | 0% (forced) | 207ms | Two spikes: 1.7s, 2.2s. |
| 4 | **Wifi → cellular mid-run** | 92% | 137ms | **0 reconnects.** One 16.2s stall. |

The phone's cellular address was `10.1.80.111/32` — a carrier-grade NAT
address, so run 2 tested the condition that matters.

## Finding 1 — CGNAT traversal works

The build plan predicted relaying as the expected outcome on cellular and
treated that as acceptable. It went **direct**, hole-punching by ping 2, and
held a direct path for 91% of pings.

This is better than planned for. Relayed operation was the fallback we were
willing to live with; on this carrier it is not even the common case.

**Caveat that has not gone away:** one carrier. Another may traverse worse.
Run 3 exists precisely because relay is the floor, and it measures out at
~207ms median — perfectly usable for notifications.

## Finding 2 — network changes don't break the connection

This was the highest-risk item in the plan, on the theory that "pair once,
ever" depends on re-finding a phone after its address changes.

**It doesn't need re-finding.** Wifi was killed mid-run at ping 6. The
connection did not drop, did not reconnect, and reported **zero reconnects**.
QUIC connection migration carried it onto cellular and it resumed on a direct
path by ping 8.

Pair-once is validated more strongly than the design claimed. The concern was
re-resolution latency; re-resolution never happens.

## Finding 3 — the real cost is a stall, not a drop

The one in-flight ping at the moment of the switch took **16.2 seconds**,
alongside `Lost connection to relay server: Ping timeout`. That is the failure
detection window, and it is the number to design around:

- The `--wait --timeout 120` default in `cli.md` is comfortably clear of it.
- A message sent at the instant a device changes network may take ~16s, not
  ~150ms. Worth surfacing in `--wait` output rather than looking hung.
- Nothing is lost — it arrives.

## Finding 4 — same-LAN connections still bootstrap via relay

Even with both devices on the same wifi, run 1 connected via `RELAY` first and
upgraded to direct a ping later. Local discovery did not win the race; connect
time was ~490ms rather than the ~1ms a LAN path would suggest.

Not a problem — it converges to direct — but it means **first-message latency
is ~500ms even on a LAN**. If that matters, mDNS discovery needs to be
explicitly configured rather than assumed. Worth revisiting at M5.

## Finding 5 — Android needs a JNI context (M6)

Running headless, iroh panics on a background thread with
`android context was not initialized` and falls back to Google's DNS:

```
Failed to read the system's DNS config, using Google DNS servers as fallback.
reason=ndk_context not initialized; call install_android_jni_context
```

Harmless here — everything worked. But **M6 must call
`install_android_jni_context`** so the app uses the system resolver instead of
hard-coded public DNS. Recorded now so it isn't rediscovered later.

## What this changes

Nothing architectural. Specifically confirmed:

- iroh as the transport — **keep**
- Relay fallback as the safety net — **keep**, measured at ~207ms
- Pair-once by public key — **validated**, more strongly than claimed
- "No server you run" — **holds**; n0's relay and DNS did the job

The design's riskiest assumption is now measured rather than assumed. M1 can
start.
