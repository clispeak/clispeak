# Signing the Android app

Read `signing.md` first if you have not: the two platforms rhyme, and one
difference between them dominates everything here.

**On macOS a signing certificate can be replaced.** Lose it, buy another, sign
the next build, and a user who already installed the app carries on.

**On Android it cannot.** The key that signs an APK *is* the app's identity to
every device that installed it. Android refuses an update signed by a
different key — not with a warning, with a flat refusal — and the only way
back is for every user to uninstall and lose their data. Which for this app
means their identity key, their spaces, and their pairings.

So the whole of this document is really one instruction with a lot of context
around it: **make the keystore, back it up in two places, and never lose it.**

| | Fixes | Costs | Reversible |
|---|---|---|---|
| **A debug key** | nothing; it is what a debug build already uses | nothing | n/a — never ship one |
| **Your own keystore** | ships an installable APK people can update | nothing | **no** |
| **Play App Signing** | Google holds the real key; yours becomes resettable | a Play Console account, and distribution through Play | yes |

## Where this app is today

`app/src-tauri/gen/android/app/build.gradle.kts` has **no `signingConfigs`
block**, so `tauri android build --apk` produces
`app-universal-release-unsigned.apk`. Android will not install that at all —
not "with a warning", it is rejected by the package manager.

Until #68, the release workflow passed `--debug`, which produced an APK signed
with the universal Android debug key. That installs, which is why it looked
fine, and it is not a thing to publish: the debug key is public, shared by
every developer on earth, and anyone can sign an update to your app with it.

So there is currently no way to hand someone an APK. That is what this closes.

## Decide this first: Play, or a download link

Everything else follows from it, and it is not really a signing question.

**Direct download** — a `.apk` on a page, which is what #25 describes. The
user has to allow installation from unknown sources, which is a real friction
and a real security lecture from their phone. You hold the key for ever. No
account, no review, no fee.

**Google Play** — a $25 one-off registration, a review process measured in
days for the first submission, a privacy policy, a data-safety declaration,
and a target-SDK treadmill Google enforces on a schedule. In exchange, users
install it the ordinary way, and **Play App Signing** means Google holds the
real signing key while you hold an *upload* key that can be reset if you lose
it. That reversibility is worth more than it sounds.

Both can be true later — an app can be on Play and also downloadable — but the
key decision has to be made before the first person installs anything, because
after that it is fixed.

**My recommendation:** make the keystore now either way, because the upload
key for Play and the signing key for a download are the same artefact, made
the same way. Deciding Play or not can wait; making the key cannot, since
nothing ships without it.

## Making the keystore

**This is yours to run, not an agent's.** It generates a private key, and it
prompts for passwords that should not exist in any transcript.

`keytool` ships with the JDK. On this machine that is
`/usr/lib/jvm/java-21-openjdk/bin/keytool`.

```bash
keytool -genkeypair -v \
  -keystore ~/voicecast-release.jks \
  -alias voicecast \
  -keyalg RSA -keysize 4096 \
  -validity 10000 \
  -storetype PKCS12
```

It asks for a store password, then your name and organisation, then confirms.
The name fields end up in the certificate and are visible to anyone who
inspects the APK; they do not have to be a legal entity, but they should not
be a joke, because they cannot be changed later either.

| Flag | Why |
|---|---|
| `-keysize 4096` | 2048 is the common example and is fine; 4096 costs nothing here and the key outlives the advice |
| `-validity 10000` | about 27 years. Play requires a certificate valid past 2033; a key that expires is a key you cannot update with |
| `-storetype PKCS12` | the modern format. Older examples produce JKS and `keytool` warns about it every time |

Then check it took:

```console
$ keytool -list -v -keystore ~/voicecast-release.jks -alias voicecast
Alias name: voicecast
Certificate fingerprints:
         SHA256: A1:B2:...
```

That SHA-256 is the app's identity. Keep a copy of it somewhere separate — it
is how you later prove which key an APK was signed with.

### Backing it up

Two copies, in two places, neither of them only this laptop, and neither of
them the repository. A password manager with file attachments is the usual
answer; an encrypted archive in a different cloud account is another. Write
the store password down in the same place, because a keystore whose password
is lost is exactly as gone as a keystore that was deleted.

This is the step people skip and the one that ends projects.

## Using it locally

The Gradle project reads `keystore.properties` if it is there, and builds
unsigned if it is not. It is gitignored at any depth under `gen/android`, and
it must stay that way.

Create it **next to `build.gradle.kts`**, at
`app/src-tauri/gen/android/app/keystore.properties`. The path matters and is
easy to get wrong: `file()` in a Gradle build script resolves against the
*module* directory, not the project root, so a copy one level up is read as
absent and the build comes out unsigned with nothing said about it. Confirmed
by putting it in both places and asking Gradle which it saw.

```properties
storeFile=/home/inpsight/voicecast-release.jks
storePassword=…
keyAlias=voicecast
keyPassword=…
```

Then:

```bash
cd app
npx @tauri-apps/cli android build --apk --target aarch64
```

and the artefact becomes `app-universal-release.apk` rather than
`…-release-unsigned.apk`. Confirm what actually signed it:

```console
$ apksigner verify --print-certs app/build/outputs/apk/universal/release/app-universal-release.apk
Signer #1 certificate DN: CN=…
Signer #1 certificate SHA-256 digest: a1b2…
```

`apksigner` is in `~/Android/Sdk/build-tools/35.0.0/`. Compare that digest to
the one `keytool` printed. If it says the APK is not signed, `keystore.properties`
was not found — check it is in `gen/android/app/` and not `gen/android/`.

The quickest way to ask Gradle directly, without building anything:

```console
$ ./gradlew :app:signingReport
Variant: universalRelease
Config: release
Store: /home/inpsight/voicecast-release.jks
Alias: voicecast
```

`Config: null` means the file was not found or `storeFile` was missing from
it. `storeFile` should be an absolute path; a relative one is resolved against
the module directory too, which is the same trap twice.

## Using it in the release workflow

`release.yml` reads four secrets and signs only when all of them are present,
exactly as the macOS job does with its three:

| Secret | Holds |
|---|---|
| `ANDROID_KEYSTORE` | the `.jks`, base64-encoded |
| `ANDROID_KEYSTORE_PASSWORD` | the store password |
| `ANDROID_KEY_ALIAS` | `voicecast` |
| `ANDROID_KEY_PASSWORD` | the key password (often the same as the store password) |

To produce the first one:

```bash
base64 -w0 ~/voicecast-release.jks > keystore.b64
```

Then in the repository: **Settings → Secrets and variables → Actions → New
repository secret**, one per row above. Paste the contents of `keystore.b64`
into `ANDROID_KEYSTORE`, then delete that file — it is the key in a form that
is easy to leak.

**With none of them set the job still builds, and says the APK is unsigned.**
That is today, and it is not an error. With all of them set the job writes the
keystore to a scratch file, points Gradle at it, builds, and **reads the
signature back with `apksigner`** before uploading — because "the build was
configured to sign and silently did not" is indistinguishable from a signed
build without asking.

The workflow deletes the decoded keystore afterwards. That matters less than
it sounds, since the runner is destroyed, and it costs nothing to be tidy.

### Why the key is in CI at all, when the macOS one is not

`signing.md` argues the self-signed macOS certificate should stay off CI,
because it buys a downloader nothing while putting a private key in a job that
runs several hundred build scripts. That reasoning does not transfer, and the
difference is worth being explicit about:

- The macOS self-signed certificate is a **development convenience**. Nobody
  else's Gatekeeper trusts it, so putting it in CI changes nothing for anyone.
- The Android keystore is the **only way to ship at all**. An unsigned APK
  does not install. There is no version of publishing Android builds that does
  not involve this key being used by whatever builds them.

So the risk is real and it is accepted rather than avoided, which is why #70's
hardening — `contents: read` on the build jobs, pinned actions, no
`npm install` fallback — mattered before this landed rather than after.

If that trade ever looks wrong, the alternative is Play App Signing, where the
key in CI is an *upload* key that Google can reset. That is the reversibility
argument again, and it is the strongest reason to choose Play.

## What signing does not do

It does not make the download warning go away. A `.apk` from a web page still
makes the phone ask whether you trust this source, whatever it is signed with,
because the warning is about where it came from and not about the signature.
Only distribution through Play removes that.

It does not verify anything about you. A self-made keystore says "the same
person made both of these builds" and nothing more, which is exactly what
Android needs and no more than that.

## The change this implies, and what is already done

- `build.gradle.kts` gained a `signingConfigs` block reading
  `keystore.properties`, and a release build that uses it when present. Done.
- `release.yml` gained the four secrets and the `apksigner` verification. Done.
- The keystore itself, the backups, and the secrets in GitHub settings are
  yours. Nothing here can do them, and nothing here should.

Related: #31 tracks this, #23 and #25 cover where the APK ends up, and #68 is
why the release job now builds the release variant at all.
