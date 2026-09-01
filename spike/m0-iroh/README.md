# M0 spike — iroh connectivity

**Throwaway.** Answers one question, then gets deleted. Do not build on it.

## The question

Can a desktop reach a phone on cellular, with no server you run — and how
often does that go direct versus relayed?

Everything in the design rests on this: pair-once, phones reachable off wifi,
no infrastructure. Relay fallback means "cannot connect" is close to ruled out
by construction, so the useful measurements are **direct-vs-relay ratio**,
**latency**, and **recovery after a network change**.

## Build

```
cargo build --release
```

## Run

One side listens, the other connects. The connector needs only the **endpoint
id** — no address, no relay hint. That is deliberate: it exercises pkarr/DNS
discovery, which is what "pair once, ever" actually depends on.

```
# device A
$ m0-iroh listen
endpoint id: 087c6fd970...

# device B
$ m0-iroh connect 087c6fd970... --pings 30 --interval 2
connected in 654ms  path: RELAY
  ping   1   108.4ms  RELAY
  ping   2     1.4ms  DIRECT+RELAY     <- hole-punched and upgraded
```

`DIRECT+RELAY` means the direct path won; iroh keeps the relay warm as a
fallback.

### Force the relay (test-matrix row 4)

```
$ m0-iroh connect <id> --force-relay
```

Removes direct IP transports so only the relay remains. This measures relayed
latency deterministically, instead of waiting for a bad network to produce it —
which is what rescues the single-carrier problem, since relay is the fallback
any awkward carrier would land on anyway.

## Getting it onto the Android phone

No app needed. This tests the *transport*, not Android's app lifecycle — those
are separate questions, and M6 is where the second one gets answered. Mixing
them means a failure tells you nothing about which half broke.

```
rustup target add aarch64-linux-android
cargo install cargo-ndk          # needs the Android NDK installed
cargo ndk -t arm64-v8a build --release

adb push target/aarch64-linux-android/release/m0-iroh /data/local/tmp/
adb shell chmod +x /data/local/tmp/m0-iroh
adb shell /data/local/tmp/m0-iroh connect <id>
```

**adb over USB keeps working while the phone is on cellular**, so you can
toggle wifi off and still drive the phone from the laptop throughout.

Alternative with no NDK: install Termux, `pkg install rust`, and build on the
phone. Simpler to set up, but compiling ~380 crates on a phone is slow.

## The test matrix

| # | Side A | Side B | What it tests |
|---|---|---|---|
| 1 | Linux, home wifi | Android, same wifi | mDNS + direct. Baseline. |
| 2 | Linux, home wifi | Android, **cellular** | CGNAT traversal. The critical case. |
| 3 | Linux, **public wifi** | Android, cellular | Both ends hostile. |
| 4 | Linux, home wifi | Android, cellular, `--force-relay` | Relay latency. |
| 5 | Linux, home wifi | Android, **toggle wifi mid-run** | Re-resolution after address change. |

Row 5 is the one to watch. Connectivity is near-guaranteed by the relay; it is
*re-finding the phone after its address changes* that has to work smoothly, or
"pair once, ever" quietly becomes "re-pair whenever you leave the house." Leave
a run going and turn wifi off — the summary reports reconnect count and total
downtime.

Set `RUST_LOG=info` (or `iroh=debug`) for iroh's own view of path selection.
