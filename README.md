# voicecast

Speak text aloud on any of your devices, from the command line.

```
$ voicecast "build finished"
$ voicecast --to pixel "needs your input"
$ voicecast --to all --priority high "deploy failed"
$ cat CHANGELOG.md | voicecast --strip --to laptop
```

Built for agents to notify you — on your desk, or on the phone in your pocket
while you're out. Devices connect **peer to peer**. There is no server to run
and no account to create.

## Status

**Working on Linux, macOS and Android.** Linux and Android run the packaged
app and talk to each other over the open internet, including on cellular.

**macOS speaks, and installs like a Mac app.** Verified on an arm64 Mac:
installed from the built dmg, synthesising through Piper, with the `voicecast`
command on the PATH. Its peer-to-peer side is exercised only as far as binding
an endpoint — pairing a Mac with another device has not been tested yet, and
an Intel Mac has no verified Piper checksum, so it is refused rather than
guessed at.

Windows and iOS build on every commit but are not yet wired to a speech
engine — the rule is that nothing merges unless it compiles for all five
targets, so adding them later is a matter of testing rather than untangling.

The riskiest assumption — that peer-to-peer connections survive carrier-grade
NAT and network changes — was [measured on real hardware](docs/m0-results.md)
before anything was built on top of it. It holds: 91% of connections went
direct, and switching between wifi and cellular caused zero reconnects.

## Installing

**Linux.** Build the Flatpak, which carries Piper and a voice so it speaks the
moment it is installed:

```bash
cd packaging/flatpak
flatpak-builder --force-clean --user --install build-dir org.voicecast.App.yml
flatpak run org.voicecast.App
```

The app installs the `voicecast` command to `~/.local/bin` on first launch, and
re-installs it whenever the app is updated. The command deliberately stays on
the host rather than inside the sandbox: entering a Flatpak costs about 86ms
against the tool's own 3ms, and an agent calls it repeatedly. They still find
each other, over an abstract socket that crosses the sandbox.

**macOS.** Build the app bundle, which carries Piper, a voice and the
command-line tool, so a drag to /Applications is the whole install:

```bash
cargo xtask bundle
open target/release/bundle/dmg/voicecast_0.1.0_aarch64.dmg
```

As on Linux, the app installs the `voicecast` command to `~/.local/bin` on
launch and rewrites it whenever the bundled copy differs, so an update cannot
leave a stale CLI behind. macOS builds its default PATH from `/etc/paths`,
which names no home directory, so the app also adds a line to `~/.zprofile` —
after `path_helper`, which would otherwise reorder it away. Nothing is written
if any of your start-up files already puts that directory on the PATH.

Builds are ad-hoc signed unless `APPLE_SIGNING_IDENTITY` names a certificate.
Ad-hoc is enough to run, at a cost worth knowing: the identity is derived from
the binary's own hash, so every rebuild looks like a different program to
macOS and the keychain grant holding the device identity is asked for again.
Any stable certificate, self-signed included, ends that.

**Android.** Build and install over USB:

```bash
cd app
ANDROID_HOME=~/Android/Sdk NDK_HOME=~/android-ndk-r29 \
  npx @tauri-apps/cli android build --apk --debug --target aarch64
adb install -r src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk
```

**Without the app.** `voicecastd` is a headless node for a machine with no
desktop. It needs Piper installed where the engine looks for it, which
`cargo xtask piper` does — `~/.local/share/voicecast` on Linux,
`~/Library/Application Support/voicecast` on macOS. Linux can instead fall
back to `espeak-ng` on `PATH`.

## Pairing

```bash
voicecast invite             # on one device — prints a ticket, app shows a QR
voicecast join <ticket>      # on the other
voicecast devices
```

Two nodes can share one machine for testing by overriding `VOICECAST_SOCKET`
and `VOICECAST_CONFIG_DIR`.

## What it does

**Speaking.** Any length. Markdown and bare URLs are *rejected* with a
suggested rewrite rather than silently mangled, so an agent can correct itself
— `--strip` converts instead, `--raw` skips the check.

**Targeting.** One device, a comma-separated list, a locally-defined group,
`all`, or `here`. Several devices are reached at once rather than one after
another.

**Priority.** `high` interrupts what is playing, says its piece, and then the
interrupted message resumes from the sentence it was cut off in. `low` is
dropped when the queue is already deep.

**Quiet.** Each device decides for itself whether it will make noise: mute, and
a daily quiet window. `high` breaks through quiet hours only if that device
allows it, and nothing breaks through mute. A sender cannot change either.

**History.** Every message a device is asked to speak is recorded, spoken or
not. A message refused while the device was muted is kept and marked unheard,
to be read or played later — playing works through mute, because pressing play
is the ask.

**Control.** `stop`, `skip`, `pause`, `resume` and `queue`, on this device or
on any other, plus in the app for whatever is playing right now.

**Spaces.** A device can belong to several separate sets of your own devices.
Bare names resolve in the default one; `work/laptop` reaches anywhere else.
There is deliberately no selector meaning "everywhere". `rotate` replaces a
space outright, which locks a lost device out immediately rather than
eventually.

See [cli.md](docs/cli.md) for the full surface and exit codes.

## Platforms

| | Speech | Background |
|---|---|---|
| Linux | Piper, falling back to espeak-ng | tray app |
| macOS | Piper | tray app |
| Android | system text-to-speech | foreground service + battery exemption |
| Windows | not yet wired | tray app |
| iOS | not yet wired | — |

Piper streams raw audio to whatever player the system has — `paplay`,
`pw-play`, `aplay`, or sox. macOS ships none that read raw audio on stdin, so
there the chunk is rendered and played with the built-in `afplay`: speech
starts a little later, and a stock Mac needs nothing installed.

## How it works

Each install is a **node** — both sender and receiver, one small Tauri app on
Linux, macOS, Windows, Android, and iOS. Devices join a **space** by scanning a
QR code once, and stay reachable afterwards even as they move between networks,
because they address each other by public key rather than by IP.

**Only text crosses the wire.** The receiving device synthesises it locally, so
five minutes of speech costs ~50KB instead of tens of megabytes, and each
device speaks in whatever voice it is configured with.

**There is no shared group secret.** Authorisation is "is this public key in my
roster?", so compromising one device leaks nothing that decrypts another's
traffic. The roster is an add-only set with tombstones, signed by whoever
invited each member, which is what lets a device admit a peer it has never met.

## Using it from an agent

`skills/voicecast/SKILL.md` is an agent skill. Install it with

```bash
voicecast skill --install                          # Claude Code's default location
voicecast skill --install --path <dir>/SKILL.md    # anywhere else
voicecast skill                                    # print it, to pipe somewhere
```

or from the app's Settings tab. The agent
gains the judgement the `--help` output cannot give it: when speaking is worth
doing at all, which device suits which kind of message, what each exit code
means for what to do next, and that a `muted` device is a decision to respect
rather than a failure to retry.

It also walks the user through a one-time working agreement — what the agent
should call itself when it speaks, which device is the default, and where the
line is between speaking and printing — and tells the agent to record it. The
naming matters once more than one agent can reach the same phone: a voice from
a pocket that does not say whose it is makes the user guess.

A test checks that every command and flag the skill mentions actually exists,
so it cannot quietly drift into describing a tool that has moved on.

## Docs

| | |
|---|---|
| [architecture.md](docs/architecture.md) | The system: identity, discovery, membership, security |
| [setup.md](docs/setup.md) | What setting up four devices actually looks like |
| [cli.md](docs/cli.md) | Command surface, exit codes, targeting |
| [protocol.md](docs/protocol.md) | Wire format and stream model |
| [text.md](docs/text.md) | Validation and chunking rules |
| [build-plan.md](docs/build-plan.md) | Milestones, repo layout, CI |
| [m0-results.md](docs/m0-results.md) | Measured transport results on real devices |
| [decisions.md](docs/decisions.md) | Every decision, with its rationale and cost |

Start with `decisions.md` if you want to know *why* rather than *what*.

## Building

Rust 1.98 or newer, and `npm install` in `app/` once.

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
cargo run -p xtask -- portability
cd app && npx tailwindcss -i src/input.css -o src/styles.css --minify
```

`cargo xtask piper` downloads Piper and a voice against pinned checksums and
puts them where the engine looks, which is what a checked-out copy needs
before it can speak. On macOS it also repairs the upstream release: that build
ships without an rpath and without the dylibs it links against, which live in
a separate `piper-phonemize` archive, so the two are merged and re-signed.

`cargo xtask bundle` stages Piper, a voice and the CLI into the app and builds
the installable bundle. Those staged files are declared in
`tauri.bundle.conf.json` rather than the main config, because Tauri's build
script checks declared resources exist on *every* `cargo check` — declaring
them normally would break `cargo build --workspace` on any machine that had
not staged them first, CI included.

The frontend is plain HTML and JavaScript with a Tailwind build step — no
framework and no bundler, because the interesting behaviour lives in
`voicecast-core`, shared with the CLI.
