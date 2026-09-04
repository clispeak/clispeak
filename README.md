# clispeak

Speak text aloud on any of your devices, from the command line.

```
$ clispeak "build finished"
$ clispeak --to pixel "needs your input"
$ clispeak --to all --priority high "deploy failed"
$ cat CHANGELOG.md | clispeak --strip --to laptop
```

Built for agents to notify you — on your desk, or on the phone in your pocket
while you're out. Devices connect **peer to peer**. There is no server to run
and no account to create.

## Status

**Working on Linux, macOS and Android.** Linux and Android run the packaged
app and talk to each other over the open internet, including on cellular.

**macOS speaks, and installs like a Mac app.** Verified on an arm64 Mac:
installed from the built dmg, synthesising through Piper, with the `clispeak`
command on the PATH. Its peer-to-peer side is exercised only as far as binding
an endpoint — pairing a Mac with another device has not been tested yet, and
an Intel Mac has no verified Piper checksum, so it is refused rather than
guessed at.

Windows now speaks with Piper, the same engine as Linux and macOS, so a
message sounds the same wherever it lands. There is no installer yet: Piper
has to be put in place with `cargo xtask piper` — a running node picks it up
within a couple of seconds, without a restart — and Windows also needs the
Microsoft Visual C++ Redistributable, which it does not ship and which Piper
links against.

iOS builds on every commit but is not yet wired to a speech engine — the rule
is that nothing merges unless it compiles for all five targets, so adding it
later is a matter of testing rather than untangling.

The riskiest assumption — that peer-to-peer connections survive carrier-grade
NAT and network changes — was [measured on real hardware](docs/m0-results.md)
before anything was built on top of it. It holds: 91% of connections went
direct, and switching between wifi and cellular caused zero reconnects.

## Installing

**Linux.** Build the Flatpak, which carries Piper and a voice so it speaks the
moment it is installed:

```bash
git submodule update --init                 # the manifest's shared-modules
npm --prefix app ci
npm --prefix app run build:css              # styles.css is generated, not committed
cargo build --release -p clispeak-app -p clispeak-cli
cd packaging/flatpak
flatpak-builder --force-clean --user --install build-dir org.clispeak.app.yml
flatpak run org.clispeak.app
```

The first four lines are not optional and used to be missing here. The
manifest takes the two binaries from `target/release`, so they have to exist;
and `cargo build` — unlike `tauri build` — does not run Tailwind, so without
the CSS step the app installs and comes up unstyled with nothing to say why.

The app installs the `clispeak` command to `~/.local/bin` on first launch, and
re-installs it whenever the app is updated. The command deliberately stays on
the host rather than inside the sandbox: entering a Flatpak costs about 86ms
against the tool's own 3ms, and an agent calls it repeatedly. They still find
each other, over an abstract socket that crosses the sandbox.

**macOS.** Build the app bundle, which carries Piper, a voice and the
command-line tool, so a drag to /Applications is the whole install:

```bash
cargo xtask bundle
open target/release/bundle/dmg/clispeak_0.1.0_aarch64.dmg
```

As on Linux, the app installs the `clispeak` command to `~/.local/bin` on
launch and rewrites it whenever the bundled copy differs, so an update cannot
leave a stale CLI behind. macOS builds its default PATH from `/etc/paths`,
which names no home directory, so the app also adds a line to `~/.zprofile` —
after `path_helper`, which would otherwise reorder it away. Nothing is written
if any of your start-up files already puts that directory on the PATH.

Builds are ad-hoc signed unless `APPLE_SIGNING_IDENTITY` names a certificate.
Ad-hoc is enough to run, at a cost worth knowing: the identity is derived from
the binary's own hash, so every rebuild looks like a different program to
macOS and the keychain grant holding the device identity is asked for again.
Any stable certificate, self-signed included, ends that — five minutes in
Keychain Access, and `docs/signing.md` has the steps. A self-signed one does
nothing for anyone *else*: a downloaded `.dmg` still gets Gatekeeper's
warning, which needs a Developer ID certificate and notarisation (#29).

**Android.** Build and install over USB:

```bash
cd app
ANDROID_HOME=~/Android/Sdk NDK_HOME=~/android-ndk-r29 \
  npx @tauri-apps/cli android build --apk --debug --target aarch64
adb install -r src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk
```

**Without the app.** `clispeakd` is a headless node for a machine with no
desktop. It needs Piper installed where the engine looks for it, which
`cargo xtask piper` does — `~/.local/share/clispeak` on Linux,
`~/Library/Application Support/clispeak` on macOS. Linux can instead fall
back to `espeak-ng` on `PATH`.

## Pairing

```bash
clispeak invite             # on one device — prints a ticket, app shows a QR
clispeak preview <ticket>   # on the other — what that code would join
clispeak join <ticket>      # ...and join it
clispeak devices
```

The destination is written *into* the ticket by whoever minted it, so the
joining device does not get to choose it. `preview` reads it out first —
locally, contacting nobody and spending nothing, so it can be run on a code
before deciding to use it. In the app, joining goes through the same two
steps: paste or scan, see which space it joins, then confirm. `join --name`
picks what to call it here.

Two nodes can share one machine for testing by overriding `CLISPEAK_SOCKET`
and `CLISPEAK_CONFIG_DIR`. `CLISPEAK_SOCKET` is a *name*, not a path — the
platform decides where it lives, and on Linux there is no file anywhere.

## What it does

**Speaking.** Any length up to 100,000 characters, about two hours of speech;
past that the receiving device refuses the message and says so, rather than
tying up its speaker for the afternoon. Markdown and bare URLs are *rejected*
with a suggested rewrite rather than silently mangled, so an agent can correct
itself — `--strip` converts instead, `--raw` skips the check.

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

An invite carries the space it was made for, so scanning it joins the one you
asked for rather than whichever happens to be the default when it is scanned.
It also carries what that space is *called*, so the joining device can name it
the same thing rather than inventing one. Joining a space *adds* it — the one
exception being the empty space a device founds for itself at first start,
which a first pairing displaces rather than leaving abandoned beside the real
one.

A space's name is local. It is how this device writes `work/laptop`, nothing
is sent when it changes, and two devices in one space may call it different
things — `join --name` and **Manage → Rename** both set only what is on the
device you run them from.

See [cli.md](docs/cli.md) for the full surface and exit codes.

## Platforms

| | Speech | Background |
|---|---|---|
| Linux | Piper, falling back to espeak-ng | tray app |
| macOS | Piper | tray app |
| Android | system text-to-speech | foreground service + battery exemption |
| Windows | Piper | tray app |
| iOS | Apple's own | **foreground only** — see below |

**iOS only speaks while the app is on screen**, and that is the platform
rather than an unfinished corner. Backgrounded, it stops answering somewhere
between five and ten minutes — measured on the simulator, which suspends less
aggressively than a real phone, so expect worse. Nothing available fixes it:
the audio background mode keeps an app alive while it is *playing*, not while
it is *waiting*, and everything that wakes a suspended iOS app needs a push
server. There is no server here, which is the point of the project.

So a phone in a pocket is exactly the case iOS cannot serve. Android can, and
does.

Piper streams raw audio to whatever player the system has — `paplay`,
`pw-play`, `aplay`, or sox. macOS ships none that read raw audio on stdin, so
there the chunk is rendered and played with the built-in `afplay`: speech
starts a little later, and a stock Mac needs nothing installed. Windows ships
none either, and none that takes a bare path, so PowerShell plays the rendered
file and is handed its path on stdin rather than on a command line.

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

`skills/clispeak/SKILL.md` is an agent skill. Install it with

```bash
clispeak skill --install                          # Claude Code's default location
clispeak skill --install --path <dir>/SKILL.md    # anywhere else
clispeak skill                                    # print it, to pipe somewhere
```

or from the app's Settings tab on a desktop. The phone build does not offer
it, because a skill is a file an agent running on that machine reads and
nothing on a phone reads one — it was offered there, and would have written
into the app's own sandbox and reported success. The agent
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
| [releasing.md](docs/releasing.md) | How binaries will be built and published, and what has to be settled first |
| [licensing.md](docs/licensing.md) | The licence, what we may redistribute, and what has to change first |

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
`clispeak-core`, shared with the CLI.

## Who can drive your node

The CLI talks to the node over a local socket, and the two prove themselves to
each other before anything is sent — a secret in the config directory, which
is kept readable only by you.

That matters because the socket *name* has no permissions on any platform here:
Linux's abstract namespace has none by design, and on macOS the socket lands in
`/tmp`. So another user on the same machine cannot make your devices speak,
read your history, or mint an invite to your space. They can still take the
name before your node does, which stops it starting — that is a nuisance, not
a leak, and it is written up in `docs/architecture.md`.

Anyone who can read your config directory can drive your node. That is the
same directory that holds your identity key, so the boundary is the same one.

**The Flatpak is packaging, not containment.** It grants itself write access
to `~/.local/bin`, which is on your PATH ahead of `/usr/bin`, and to
`~/.claude`, which holds hooks your agent runs. Either is enough to escape the
sandbox, and they are there because an app that offers to install a tool and
then silently fails to is worse than one that says what it can reach. Install
it because it is convenient, not because it is contained.

## Licence

**MIT OR Apache-2.0**, at your option — the Rust ecosystem's usual pair.
Apache-2.0 carries an explicit patent grant; MIT is there for anyone who
prefers the shorter terms. See [LICENSE-MIT](LICENSE-MIT) and
[LICENSE-APACHE](LICENSE-APACHE).

**The speech engine is not ours and is not covered by that.** Piper, its
phonemiser and ONNX Runtime are MIT; **espeak-ng is GPL-3.0-or-later**; and a
voice model carries the terms of the corpus it was trained on, which are
frequently *not* redistributable. `cargo xtask piper` downloads all of it onto
your own machine, which is yours to do.

**Distributed builds are a different question and it is not finished.** The
default voice today is trained on a research-only corpus that bars
redistribution, so it has to change before anything is published, and the
speech payload should be fetched on first run rather than bundled. Nothing has
been published yet. [licensing.md](docs/licensing.md) has the working, and it
is a careful reading of licence text by people who are not lawyers.
