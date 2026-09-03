# Working on voicecast

A command-line tool that speaks text aloud on your devices, peer to peer, with
no server. Each install is a node — sender and receiver both. The CLI is a
thin client that hands text to the local node and exits.

**The point of the project is that an *agent* drives the CLI.** That shapes
almost every decision: distinct exit codes, errors that carry a fix, and
receivers that report a reason rather than swallowing a failure.

## The one rule that matters most

> **If it doesn't compile for all five targets, it doesn't merge.**
> linux · android · macos · ios · windows

**Local gates are not that gate.** `cargo test`, `cargo clippy` and
`cargo fmt` only exercise the machine they run on. Passing them says nothing
about four of the five targets. Windows was broken for fifteen commits while
local checks reported green every time — see issue #5. Check the CI run, not
your terminal, before claiming a change is safe.

`cargo run -p xtask -- portability` fails if `voicecast-proto`,
`voicecast-text` or `voicecast-core` grows a platform conditional — any of
`target_os`, `target_family`, `target_arch`, `target_env`,
`target_pointer_width`, `unix` or `windows`, however it is wrapped. Platform
divergence belongs in `voicecast-engine` or the Tauri shell. Anything that
cannot be expressed portably gets a trait, not a conditional in the middle of
business logic.

For a long time it checked only the first two of those, so `cfg(unix)` sat in
`voicecast-core` while the gate printed "3 crates clean" (#88). Where there is
genuinely no portable spelling — setting a file mode is the only case so far —
say so on the line above and the gate counts it instead of failing:

```rust
// portability-exception: a file mode has no portable spelling
#[cfg(unix)]
```

The reason is required, and the count is printed, because an exemption nobody
reads is the same silence the gate exists to break.

**The divergence that has actually bitten was undeclared.** `portability`
finds platform code that *announces itself* with a `cfg`. Every one of these
compiled on all five targets and passed every gate:

| | |
|---|---|
| `/etc/hostname` | a Linux file, so macOS and Windows called every device "this device" (#38) |
| `Command::new("npx")` | resolves `npx.cmd` on no Windows box |
| `target/release/voicecast` | has no `.exe` |
| `sun_path` | a socket *name* over ~83 bytes refuses to bind on macOS, and the platform prepends a prefix you did not write |
| `isMinifyEnabled` | true only for release, so R8 deleted the Kotlin the engine calls (#41) |

The socket row says *name*, not path, on purpose: it is a namespaced name, the
platform decides where it lands, and on Linux there is no file anywhere (#43,
decision 35). The budget is measured rather than documented — 83 bytes bound
and 84 refused on a Mac, plus a `/tmp/` prefix confirmed with `lsof`, against a
documented `sun_path` of 104. The missing 16 bytes are unexplained, probably
headroom inside `interprocess`'s own check. Measure against 83; it is the
number that decides whether a node starts.

A file path, a binary name, a separator, a length limit, a build flag. None is
platform-*shaped*; a `cfg` on any of them would have been louder and would
have been caught. When you write one, ask what the other four targets have
there — and note the last row is not a platform difference at all. It is a
difference between two builds for the same platform, and it fooled us the
same way, because everything anyone had run on a real phone was a debug build.

**Compiling on five is a weaker claim than it reads as.** `cargo test`
runs once, on `ubuntu-latest`. The five matrix jobs only build. So a test
asserting cross-platform behaviour is executed on Linux and nowhere else, and
"CI green" means *compiles* on five, *tested* on one. That is a deliberate
trade — macOS runners bill at ten times Linux — but say "build-verified" when
that is what happened.

## Where things live

| Crate | Holds | Must stay portable |
|---|---|---|
| `voicecast-proto` | Wire and IPC types. Pure data, CBOR | yes |
| `voicecast-text` | Validation, chunking. Pure functions | yes |
| `voicecast-core` | Transport, roster, spaces, queue, policy, history | **yes** |
| `voicecast-engine` | `SpeechEngine` trait and per-platform impls | no |
| `voicecast-cli` | The `voicecast` binary. Depends only on proto + text | — |
| `voicecast-daemon` | Headless node, for machines with no desktop | — |
| `voicecast-keystore` | Platform keyring | — |
| `app/src-tauri` | The app: owns a `Node`, adds a tray and a window | no |
| `app/src` | Frontend. Plain HTML and JS, Tailwind build step | — |

`voicecast-cli` depending on only two crates is deliberate — it keeps startup
at ~3ms, which is the whole premise of the thin-client design. Do not reach
for `voicecast-core` from it; duplicate the handful of bytes instead, as
`frame.rs` and the socket name already do, and keep them in step by hand.

## Checks

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p xtask -- portability
cargo test --workspace
cd app && npx tailwindcss -i src/input.css -o src/styles.css --minify
```

`app/src/styles.css` is generated and **not** committed. Tauri's
`beforeBuildCommand` rebuilds it, so `tauri build` and `tauri android build`
are covered — but a plain `cargo build -p voicecast-app`, which is how the
Flatpak is packaged, is not. Add a class in `index.html`, package it that way,
and the class is inert with nothing to say so.

**On a memory-constrained machine**, wrap cargo so an OOM kills the build and
not your session: `systemd-run --user --scope -p MemoryMax=3G -p
MemorySwapMax=0 cargo …`. `.cargo/config.toml` already limits jobs and
debuginfo for the same reason.

## Running it

```bash
cargo run -p voicecast-daemon          # a node
cargo run -p voicecast-cli -- status
```

Two nodes on one machine: override `VOICECAST_SOCKET` and
`VOICECAST_CONFIG_DIR`. Both are read by the CLI *and* the node, and
forgetting the socket override makes every command talk to the first node,
which looks like two unrelated bugs.

**`VOICECAST_SOCKET` is a name, not a path, and it must be short** — under
about 83 bytes on macOS, where the platform then prepends `/tmp/`. Point it at
a scratch directory and the node refuses to bind for a reason that names the
limit and not the string. Scratchpad paths here are around 100 bytes on their
own, so this is the default way to hit it, not an edge case. Use
`VOICECAST_SOCKET=vc-2.sock`, and put only `VOICECAST_CONFIG_DIR` somewhere
deep.

Platform packages: `packaging/flatpak/` for Linux, `cargo xtask bundle` for a
macOS `.app`, and `npx @tauri-apps/cli android build --apk` for Android.
`cargo xtask piper` puts Piper and a voice where the engine looks.

## Conventions

**A limitation is a debt in three places.** When something cannot yet be done,
it gets written into the README and said in the interface as well as tracked as
an issue. Whoever removes the limitation owes all three in the same change —
otherwise the code gains a capability while two documents go on denying it. That
has already happened once here, one commit apart, and was caught by luck rather
than process.

**Docs are part of the change.** `docs/decisions.md` is numbered and
append-only: a decision records what was chosen, *why*, and what it costs.
`docs/build-plan.md` tracks milestones. If a change alters behaviour the docs
describe, the docs move with it.

**Errors are written for whoever reads them.** A rejected message shows the
offending span and a rewrite that can be sent verbatim; a device that will not
speak says which policy stopped it. An agent that can only be told "no" will
guess.

**Report what happened.** A receiver that cannot speak returns a status and a
reason, never silence and never a lie. The same applies to saying a change is
done: if the tests were not run, say so.

## Testing on real devices

Linux and Android get real runtime testing; the rest are build-verified. The
things no CI can cover are exactly the ones that have bitten: NAT traversal,
Android doze, network switching, audio actually coming out of a speaker, and
sockets left behind by a node that died. When a change touches those, test it
on hardware and say which hardware.

**Test the build you intend to ship.** Android's release build minifies and its
debug build does not, so for months "tested on a phone" meant a build nobody
would ever download — and the first release APK to reach a real phone died on
launch (#41). Before a release, install the *release* artefact and open it. The
same question applies to any packaged form: the Flatpak is `cargo build`, not
`tauri build`, and does not regenerate the CSS.
