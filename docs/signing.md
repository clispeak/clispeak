# Signing the macOS app

Two different problems wear the same word. Keeping them apart is most of the
decision, because the cheap fix solves one of them completely and the other
not at all.

| | Fixes | Costs |
|---|---|---|
| **A self-signed certificate** | the keychain prompt on every rebuild | nothing |
| **A Developer ID certificate** | Gatekeeper's warning for other people | a paid Apple Developer membership |

## Why a rebuild asks for the keychain password

The bundle is *ad-hoc* signed, which means it has no certificate and its
identity is its own hash. The consequence is visible in one command:

```console
$ codesign -d -r- /Applications/voicecast.app
# designated => cdhash H"24a1051019672ae35fecd1b28ff085f7ca6edbff"

$ codesign -d -r- /Applications/Safari.app
designated => identifier "com.apple.Safari" and anchor apple
```

That line is the *designated requirement* — how macOS decides whether a
program is the same program as last time. Safari's names an identifier and an
issuer, so any build Apple signs satisfies it. Ours names one specific build.

A keychain grant stores that requirement. So **Always Allow** on the
`voicecast / device-identity` item names a hash, and the next `cargo xtask
bundle` produces a different hash, so the grant no longer matches and the
password is asked for again. The node reads that item *before* it binds its
socket, so while the dialog is up the app looks launched, the process is
alive, and `voicecast status` reports no node running and sends you off to
start a second one (#19, #29).

A certificate changes the requirement to a name and an issuer. The grant then
survives every later build, because every later build satisfies it.

## Creating one (self-signed, for this machine)

**This is yours to run, not an agent's** — it writes a private key into your
login keychain, and that is a thing you should watch happen.

Keychain Access → *Certificate Assistant* → **Create a Certificate…**

| Field | Value |
|---|---|
| Name | `voicecast dev` |
| Identity Type | Self Signed Root |
| Certificate Type | **Code Signing** |
| Let me override defaults | not needed |

Then check it took:

```console
$ security find-identity -v -p codesigning
  1) A1B2C3…  "voicecast dev"
     1 valid identities found
```

Zero identities means the certificate type was not Code Signing — that is the
one field the dialog defaults wrong.

## Using it

One variable covers the whole bundle: Tauri signs the app with it, and
`xtask bundle` signs the Piper binaries and dylibs beside it with the same
one.

```bash
export APPLE_SIGNING_IDENTITY="voicecast dev"
cargo run -p xtask -- bundle
```

Put that `export` in `~/.zprofile` so it is not a thing to remember, then
confirm the requirement changed shape:

```console
$ codesign -d -r- target/release/bundle/macos/voicecast.app
designated => identifier "com.voicecast.app" and certificate leaf H"…"
```

Install it, answer the keychain dialog **once** with Always Allow, then
rebuild and reinstall. The second launch should not ask. If it does, the
certificate is not being picked up — check `codesign -dv` says
`Authority=voicecast dev` rather than `Signature=adhoc`.

## What a self-signed certificate does not do

Nobody else trusts it. A `.dmg` downloaded from the release page still gets
Gatekeeper's "cannot be opened because the developer cannot be verified",
whether it is ad-hoc or signed by a certificate only this Mac has ever seen.
Distribution needs a **Developer ID Application** certificate from a paid
Apple Developer membership, and notarisation on top of it — a separate
question, tracked in the same milestone.

So the self-signed certificate is a *development* fix. It ends the rebuild
prompt, which is the whole of what #29 diagnosed, and it is worth doing today
because it costs nothing.

## The release workflow

`release.yml` reads three secrets and signs only if they are all present:

| Secret | Holds |
|---|---|
| `APPLE_CERTIFICATE` | the `.p12`, base64-encoded |
| `APPLE_CERTIFICATE_PASSWORD` | the password set when exporting it |
| `APPLE_SIGNING_IDENTITY` | the certificate's name, e.g. `Developer ID Application: … (TEAMID)` |

The Tauri CLI does the keychain work itself — `security create-keychain`,
`import -T /usr/bin/codesign`, `set-key-partition-list`, and
`delete-keychain` at the end — so there is no keychain script here to get
wrong.

**With none of them set the job still builds, and says the artefact is
unsigned.** That is the useful default while there is no certificate to use,
and it is why the job *verifies the signature it produced* rather than
assuming the variables took: a build configured to sign that silently did not
is the failure this repository keeps meeting, and it is indistinguishable from
a signed one without reading the designated requirement.

### Exporting the `.p12`, when there is a Developer ID to export

Keychain Access → the certificate → right-click → **Export…** → Personal
Information Exchange (.p12), with a password. Then:

```bash
base64 -i voicecast.p12 | pbcopy    # paste into the APPLE_CERTIFICATE secret
```

**Do not put the self-signed certificate here.** It would buy a downloader
nothing — the warning is identical — while putting a code-signing private key
inside a job that runs several hundred crates' build scripts, npm lifecycle
scripts and a Gradle plugin, any of which can read the environment. That is
the surface #70 was about, and a signing key is a worse thing to lose there
than a token, because a token can be revoked without anyone having installed
something.
