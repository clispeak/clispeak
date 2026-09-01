# Build plan

**Phase 1 targets Linux and Android.** They get real runtime testing. macOS,
Windows, and iOS are **build-verified from day one but not exercised** — the
point is that adding them later is a matter of testing, never of untangling
architecture.

---

## The rule that keeps later targets cheap

> **If it doesn't compile for all five targets, it doesn't merge.**

CI enforces this from the first commit, even though three targets are never
run. The failure mode this prevents is well known: six months of Linux-shaped
assumptions quietly accumulating in shared code, discovered the week someone
tries to build for iOS.

Concretely:

- `tts-proto`, `tts-text`, and `tts-core` contain **no** `#[cfg(target_os)]`.
- All platform divergence lives in `tts-engine` and the Tauri shell.
- Anything that can't be expressed portably gets a trait in `tts-core` and an
  implementation in `tts-engine`, not a conditional compile in the middle of
  business logic.

## Repo layout

```
tts/
├── Cargo.toml                  workspace
├── crates/
│   ├── tts-proto/              CBOR wire types + IPC types. Pure data.
│   ├── tts-text/               validation, protection, chunking. Pure fns.
│   ├── tts-engine/             SpeechEngine trait + per-platform impls.
│   ├── tts-core/               iroh transport, roster, queue, playback.
│   └── tts-cli/                the `tts` binary. Thin: proto + text only.
├── app/
│   ├── src/                    Tauri frontend
│   └── src-tauri/              Tauri shell, depends on tts-core
├── xtask/                      build automation
└── docs/
```

Each crate has an obvious test story, which is the reason for the split:

| Crate | Testable how |
|---|---|
| `tts-proto` | Round-trip encode/decode, version-skew cases |
| `tts-text` | Table-driven unit tests. Every ugly case in `text.md` is a row. |
| `tts-engine` | Per-platform smoke tests; trait conformance |
| `tts-core` | Two nodes in one process over an in-memory transport |
| `tts-cli` | Golden-file tests on stdout/stderr and exit codes |

`tts-cli` depending only on `tts-proto` and `tts-text` is deliberate — it keeps
the binary small and its startup instant, which is the whole premise of the
thin-client design.

## CI from the first commit

| Runner | Builds | Tests |
|---|---|---|
| ubuntu | linux, android (`cargo-ndk`) | full |
| macos | macos, ios | build only |
| windows | windows | build only |

Three runners cover five targets. macOS/Windows/iOS turn red the moment
someone writes non-portable code, which is the entire point.

---

## Milestones

### M0 — Spike: does iroh actually work on carriers?

**The riskiest assumption in the design, tested before anything is built on
it.** Pair-once, phones reachable off-wifi, and "no server you run" all rest
on iroh's hole-punching and pkarr discovery behaving as advertised on real
mobile networks — which are almost universally CGNAT.

Throwaway code. **Do not build on it.**

*Exit criteria:*
- Linux box on home wifi connects to an Android phone on cellular
- Repeated on **two different carriers**
- Record: connection success rate, direct-vs-relayed ratio, time to first
  byte, behavior when the phone switches wifi ↔ cellular mid-connection

*What would change course:* if connections fail often, reconsider the
transport. If they succeed but always relay, that's **acceptable** — note the
added latency and move on. Relayed is the expected case on cellular.

### M1 — Skeleton and CI

Workspace, five-target CI matrix, `xtask`, licence, lint and format gates.
Nothing functional.

*Exit:* an empty workspace builds green for all five targets.

### M2 — `tts-text`

Validation and chunking. Pure functions, no I/O, no async. Every ugly case in
`text.md` becomes a test row.

Deliberately first among the real crates: highest test density, zero risk, and
it needs no decisions from anywhere else.

*Exit:* markdown and URL rejection with correct spans and suggestions;
protection-pass splitting handles `10.0.0.1`, `src/main.rs:42`, `Dr.`,
`v1.2.3`; fallback cascade never splits mid-word.

### M3 — Speak locally

CLI → unix socket → node → `tts-engine` → sound. No network, no identity, no
roster.

*Exit:* `tts "hello world"` speaks on the same Linux machine through
espeak-ng. `echo x | tts` works. Exit codes correct. **First demoable thing.**

### M4 — Identity and a space of one

Keypair generation, keyring storage, `tts init`, `tts status`.

*Exit:* identity survives a restart; the keyring is used, not a bare file.

### M5 — Two Linux machines

iroh transport, control stream, `Hello`, roster, `SpeakBegin`/`Chunk`/
`SpeakEnd`, `Status` back. Join by pasted ticket — QR can wait.

*Exit:* two Linux boxes on a LAN join a space and speak to each other; then
the same across networks, exercising the relay path from M0.

### M6 — Android

Tauri v2 mobile shell, `TextToSpeech` engine, foreground service, QR scanning,
join flow, receiver settings UI.

**This is where carrier testing happens for real** — M0 proved the transport,
this proves it inside a real app with a real lifecycle: doze mode, network
changes, process death, app backgrounding.

*Exit:* the four-device walkthrough in `setup.md` works for Linux + Android;
the phone receives on cellular after being idle for hours.

### M7 — Voices and fallback

Piper download with pinned URLs and checksums, engine tiering, reason codes,
fallback UI on both platforms, `Presence` reporting.

*Exit:* a fresh Linux install speaks via espeak immediately, shows *why* it is
in fallback, and upgrades to Piper without a restart.

### M8 — The rest of the CLI

Priority and queue, `stop`/`skip`/`pause`, groups, multiple spaces, quiet
hours, `--wait`/`--json`, rotation.

*Exit:* `cli.md` is fully implemented.

---

## Testing strategy

**Unit** — heaviest in `tts-text` and `tts-proto`. Roster CRDT merge is a good
property-test target: merge must be commutative, associative, idempotent.

**Integration** — two `tts-core` nodes in one process over an in-memory
transport. Covers roster convergence, queue behavior, and cancellation without
touching a network.

**Manual, and unavoidable** — real devices for CGNAT traversal, doze mode,
network switching, and audio output. No CI substitute exists for these, which
is exactly why M0 comes first.

## Deferred past phase 1

macOS, Windows, and iOS runtime testing · `iroh-blobs` voice sync between
devices · cloud/API voices · smart speakers.

All of these are additive. None require revisiting a phase 1 decision — which
was the goal of the compile-for-five rule.
