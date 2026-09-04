# Signing the macOS app

Two different problems wear the same word, and **one certificate solves both**
— which is not what this file said until someone tried it.

| | Fixes | Costs |
|---|---|---|
| A self-signed certificate | ~~the keychain prompt on every rebuild~~ **nothing, measured** | nothing |
| **A Developer ID certificate** | the rebuild prompt (**measured**) *and* Gatekeeper's warning (needs notarisation) | a paid Apple Developer membership |

The rebuild prompt result, on macOS 26.5:

| signing | designated requirement | rebuild |
|---|---|---|
| ad-hoc | `cdhash H"24a1051…"` | prompts |
| self-signed | `identifier … and certificate leaf = H"…"` | **prompts** |
| Developer ID | `identifier … and anchor apple generic and certificate leaf[subject.OU] = …` | **silent, twice** |

Two consecutive rebuilds, each `CDHash` verified different before installing
so neither could pass vacuously. `anchor apple generic` is the difference: a
trusted anchor is what a keychain ACL can name.

**Both halves of that table are now measured on macOS 26.5**, one negative and
one positive, on the same Mac within an hour of each other. The self-signed
route was tried and does not work. The Developer ID route was tried and does.

The self-signed section is kept because the mechanism it explains is the
mechanism both routes rely on, and because a negative result is worth more
written down than deleted.

### What was actually measured

On macOS 26.5, with a self-signed Code Signing certificate:

- `codesign` used it without complaint — `Authority=clispeak dev`, and
  `--verify --deep` reported `satisfies its Designated Requirement`.
- The designated requirement changed from `cdhash H"…"` to
  `identifier "org.clispeak.app" and certificate leaf = H"…"`, which is the
  stable shape this file said was the point.
- **The keychain still prompted on every rebuild anyway.** Three builds, each
  with a genuinely different `CDHash`, each answered with *Always Allow*, each
  followed by another prompt on the next one. The process was parked on the
  same stack as the original diagnosis in #29:
  `SecKeychainFindGenericPassword` → `ClientSession::decrypt`.

So the requirement being stable is necessary and is not sufficient. The
untested explanation is that a keychain ACL needs a *trusted* anchor to name,
and macOS does not trust a self-signed root — the same fact that makes
`find-identity -v` report zero. A Developer ID certificate chains to Apple's
root and has no such problem, which is why it is the answer for both halves
and why nobody should spend an afternoon on the free one first.

## Why a rebuild asks for the keychain password

The bundle is *ad-hoc* signed, which means it has no certificate and its
identity is its own hash. The consequence is visible in one command:

```console
$ codesign -d -r- /Applications/clispeak.app
# designated => cdhash H"24a1051019672ae35fecd1b28ff085f7ca6edbff"

$ codesign -d -r- /Applications/Safari.app
designated => identifier "com.apple.Safari" and anchor apple
```

That line is the *designated requirement* — how macOS decides whether a
program is the same program as last time. Safari's names an identifier and an
issuer, so any build Apple signs satisfies it. Ours names one specific build.

A keychain grant stores that requirement. So **Always Allow** on the
`clispeak / device-identity` item names a hash, and the next `cargo xtask
bundle` produces a different hash, so the grant no longer matches and the
password is asked for again. The node reads that item *before* it binds its
socket, so while the dialog is up the app looks launched, the process is
alive, and `clispeak status` reports no node running and sends you off to
start a second one (#19, #29).

A certificate changes the requirement to a name and an issuer. The grant then
survives every later build, because every later build satisfies it.

## Creating one (self-signed, for this machine)

**This is yours to run, not an agent's** — it writes a private key into your
login keychain, and that is a thing you should watch happen.

Keychain Access → *Certificate Assistant* → **Create a Certificate…**

| Field | Value |
|---|---|
| Name | `clispeak dev` |
| Identity Type | Self Signed Root |
| Certificate Type | **Code Signing** |
| Let me override defaults | not needed |

Then check it took. This is the whole output on macOS 26, captured rather
than composed:

```console
$ security find-identity
     1 identities found

  Valid identities only
     0 valid identities found
```

**Both halves of that are expected, and the second one is why this section
had to be rewritten twice.**

`1 identities found` is the answer: the certificate and its private key are in
the keychain and paired. Note that it does *not* print the identity's name —
`find-identity` only lists the valid ones, and a self-signed certificate is
not one, so there is no line to read the name off. You know the name because
you typed it into the dialog.

`0 valid identities found` is *not* a failure. `-v` filters to identities the
trust store calls valid, and macOS does not trust a self-signed root — by
design, permanently, no matter what you do. This file used to say that zero
identities meant the certificate type was wrong. It ran on the machine it was
written for, said the setup had failed, and sent the reader looking for a
mistake that was not there. The certificate was correct and signed the app on
the first attempt.

Trust and usability are different questions. `codesign` needs the certificate
and its private key, which is what the unfiltered command reports.
Gatekeeper's opinion of a self-signed root is settled — it does not trust it —
and that is expected, not a fault to fix.

So the check that actually answers the question is to sign something:

```console
$ codesign --force --sign "clispeak dev" /tmp/probe
$ codesign -dv --verbose=2 /tmp/probe 2>&1 | grep Authority
Authority=clispeak dev
```

If that says `Authority=clispeak dev`, it is working. If it says
`Signature=adhoc`, the identity never reached `codesign`. And if
`find-identity` with no flags reports zero, *then* the certificate type was
not Code Signing — that is the one field the dialog defaults wrong.

(Section 2 below uses `-v` deliberately, because a Developer ID certificate
chains to Apple's trusted root and is therefore valid as well as usable. That
is reasoning about a case nobody has run yet, and section 2 says so where it
says it.)

## Using it

One variable covers the whole bundle: Tauri signs the app with it, and
`xtask bundle` signs the Piper binaries and dylibs beside it with the same
one.

```bash
export APPLE_SIGNING_IDENTITY="clispeak dev"
cargo run -p xtask -- bundle
```

Put that `export` in `~/.zprofile` so it is not a thing to remember, then
confirm the requirement changed shape:

```console
$ codesign -d -r- target/release/bundle/macos/clispeak.app
designated => identifier "org.clispeak.app" and certificate leaf H"…"
```

Install it, answer the keychain dialog **once** with Always Allow, then
rebuild and reinstall. The second launch should not ask. If it does, the
certificate is not being picked up — check `codesign -dv` says
`Authority=clispeak dev` rather than `Signature=adhoc`.

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

`release.yml` signs in a **separate job** from the one that builds, so the
certificate is never in the environment of a step that runs `npm ci` and
several hundred crates' build scripts. See decision 101 and #117.

| Secret | Holds |
|---|---|
| `APPLE_CERTIFICATE` | the `.p12`, base64-encoded |
| `APPLE_CERTIFICATE_PASSWORD` | the password set when exporting it |

**There is no `APPLE_SIGNING_IDENTITY` secret any more.** The signing job
imports the certificate and then asks the keychain which identity arrived,
requiring exactly one `Developer ID Application`, and signs with its SHA-1
hash. The first real run failed on `no identity found` with the import
having plainly worked — and the name was a secret, so every log line
carrying it was masked and the failure read identically whether the string
was wrong, quoted or padded. A value the certificate already knows is not
worth asking a person for.

That job does the keychain work itself: `security create-keychain`,
`import -T /usr/bin/codesign`, `set-key-partition-list`, and
`delete-keychain` on the way out under `if: always()`.

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
base64 -i clispeak.p12 | pbcopy    # paste into the APPLE_CERTIFICATE secret
```

**Do not put the self-signed certificate here.** It would buy a downloader
nothing — the warning is identical — while putting a code-signing private key
inside a job that runs several hundred crates' build scripts, npm lifecycle
scripts and a Gradle plugin, any of which can read the environment. That is
the surface #70 was about, and a signing key is a worse thing to lose there
than a token, because a token can be revoked without anyone having installed
something.

---

# The paid half: Developer ID and notarisation

Everything above ends the rebuild prompt on this Mac. None of it helps anyone
who downloads the app. This half does, and it costs money and about a day of
waiting.

Two artefacts are needed and they are not the same thing. **Signing** with a
Developer ID certificate says who built the app. **Notarisation** is Apple
scanning that signed app and issuing a ticket saying they have seen it.
Gatekeeper wants both. A Developer ID signature without notarisation is still
refused, so there is no half-way state worth stopping at.

## 1. Enrol in the Apple Developer Program

<https://developer.apple.com/programs/> — 99 USD a year, renewed annually or
the certificates stop being trusted for new downloads.

Choose **Individual** unless there is a reason not to. An Organization
enrolment needs a D-U-N-S number for the legal entity and takes days to weeks
longer; it exists so the certificate carries a company name rather than a
person's. That is a licensing-and-identity decision, not a technical one, and
it interacts with the still-open question of what this project is (#24). An
Individual enrolment can be moved to an Organization later.

Enrolment is not instant. Budget for it.

## 2. Create a Developer ID Application certificate

**Developer ID Application** is the one. Not "Apple Development", not "Apple
Distribution", not "Mac App Distribution" — those are for Xcode builds and for
the App Store, and Gatekeeper will not accept them for a direct download.
There is also a **Developer ID Installer** certificate, which signs `.pkg`
installers; we ship a `.dmg` and a `.app`, so it is not needed.

Easiest path, on the Mac that will hold the key:

Xcode → **Settings** → **Accounts** → select the Apple ID → **Manage
Certificates…** → **+** → **Developer ID Application**.

Without Xcode, generate a certificate signing request in Keychain Access
(**Keychain Access** → menu → **Certificate Assistant** → **Request a
Certificate From a Certificate Authority**, saved to disk) and upload it at
<https://developer.apple.com/account/resources/certificates>.

**Apple limits a team to five Developer ID Application certificates, and
revoking one invalidates every app signed with it that has not been
notarised.** Notarised builds survive revocation, because the ticket vouches
for them independently. Make one, back up the private key, and do not make a
second to "test something".

Back it up the same way as the Android keystore: export the certificate *and
its private key* as a `.p12` from Keychain Access, and keep it somewhere that
is not this laptop. Losing it is not as final as losing an Android signing key
— you can make another, four times — but it is not free either.

Confirm what you have:

```bash
security find-identity -v -p codesigning
```

Captured on macOS 26.5, the first time anyone ran it:

```console
$ security find-identity -v -p codesigning
  1) 8C73BFFFE78B52E605D15CA1F1CE20428EE5B434 "Developer ID Application: Patrick Hogg (KC7NLB7CX8)"
     1 valid identities found
```

That quoted string, quotes excluded, is `APPLE_SIGNING_IDENTITY`.

`-v` is right *here*, unlike in the self-signed section above, and the
unfiltered command shows why in macOS's own words. With both certificates in
one keychain:

```console
$ security find-identity
  1) A494CDD8744573039B35C14E2FE6953AEEE8DA26 "clispeak dev" (CSSMERR_TP_NOT_TRUSTED)
  2) 8C73BFFFE78B52E605D15CA1F1CE20428EE5B434 "Developer ID Application: Patrick Hogg (KC7NLB7CX8)"
     2 identities found
```

`CSSMERR_TP_NOT_TRUSTED`. The self-signed certificate was never malformed; it
was untrusted, which is exactly the thing a keychain ACL cannot anchor to. The
two certificates differ by one word from the operating system, and that word
is the whole of #29.

## 3. Get notarisation credentials

There are two ways to authenticate, and `tauri build` accepts either. **Use
the API key.** Both were read out of `tauri-bundler` 2.9.4 rather than out of
documentation.

| | Variables | What it is |
|---|---|---|
| **API key** *(use this)* | `APPLE_API_KEY`, `APPLE_API_ISSUER`, `APPLE_API_KEY_PATH` | a scoped, revocable key belonging to the team |
| App-specific password | `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID` | a password attached to your personal Apple ID |

The API key wins for one reason: it can be revoked on its own. An
app-specific password is a credential on the Apple ID that owns the
membership, and the blast radius of losing one is the account rather than the
key. `APPLE_TEAM_ID` is **required** on the password route — the bundler
raises a hard error without it, while every other missing-credential case only
warns.

To create the key: App Store Connect → **Users and Access** → **Integrations**
→ **App Store Connect API** → generate a key. The *Developer* role is enough
to notarise.

That page gives you three things, and one of them exactly once:

- **Key ID** — a short string. This is `APPLE_API_KEY`.
- **Issuer ID** — a UUID, shown once per team at the top of the page. This is
  `APPLE_API_ISSUER`.
- **`AuthKey_<KEYID>.p8`** — **downloadable once.** Lose it and you revoke the
  key and make another. Back it up when you download it, not later.

### Where the `.p8` has to be

This is the part that is easy to get wrong, and it differs between your Mac
and CI.

`tauri-bundler` never hands the key's *contents* to `notarytool`; it only ever
passes a **path**. So the file has to exist on disk before the build runs, and
nothing places it for you.

**Set all three variables, on your Mac as well as in CI.** Keep the `.p8`
wherever you like — `~/.appstoreconnect/private_keys/` is where Apple's own
tools look, so it is a reasonable home — and point `APPLE_API_KEY_PATH` at it
explicitly.

There *is* a fallback. With `APPLE_API_KEY` and `APPLE_API_ISSUER` set and the
path variable unset, the bundler searches `./private_keys`,
`~/private_keys`, `~/.private_keys` and `~/.appstoreconnect/private_keys` for
`AuthKey_<KEYID>.p8`. Do not build the instruction on it. When the search
misses, you land in step 3 of the next section: notarisation is skipped, a
warning goes into the build log, and a signed but un-notarised app comes out.
Naming the path makes a missing key loud instead of quiet, and it makes your
Mac and CI the same shape rather than two stories that read as a
contradiction later.

**In CI there is no home directory to have put it in**, so the release job
writes the file out of a secret and sets `APPLE_API_KEY_PATH` to where it
wrote it, exactly as the Android job does with the keystore. It is removed
again on every path out of the job.

## 4. What `tauri build` actually does with all this

Read out of `tauri-bundler` 2.9.4 rather than assumed, because the order
matters and the failure modes are quiet:

1. If no signing identity is given, **nothing** below happens — no signing
   and therefore no notarisation. (This section describes `tauri build`,
   which is still how a *local* signed build works. In CI the bundler is no
   longer given a certificate at all; the `macos-sign` job signs afterwards.)
2. The app is signed inside out: sidecars and frameworks first, the bundle
   last. Hardened runtime is on by default, which notarisation requires.
3. Notarisation credentials are looked up. **If they are missing, the build
   logs a warning and carries on**, producing a signed but un-notarised app.
   The one exception is an Apple ID and password with no team ID, which is a
   hard error.
4. The `.app` is submitted, and on success the ticket is **stapled** to it.
5. The `.dmg` is built around the stapled app and signed — but the `.dmg`
   itself is not submitted and not stapled.

Step 3 is why the release job checks afterwards instead of trusting the build.
A warning in three hundred lines of build log is not a report.

### Step 5 is a gap, and it has a fix

Say it plainly: **the disk image is assessed by Gatekeeper, and no ticket for
it exists anywhere** — not stapled to it, and not in Apple's database, because
Tauri never submits it.

Measured on a Mac rather than reasoned about. A downloaded `.dmg` carries
`com.apple.quarantine`, and the attribute *propagates* to the `.app` copied
out of it. So both are assessed. The app is fine — it carries its own stapled
ticket, which is exactly what lets it pass offline. The disk image has
nothing.

`stapler` will tell you, and it works on a `.dmg`:

```bash
xcrun stapler validate clispeak_0.1.0_aarch64.dmg
# → "does not have a ticket stapled to it"
```

What that costs a downloader in practice is the only untested part left, and
it is a much narrower question than "is opening the dmg clean".

**It is fixable rather than merely regrettable.** `xcrun notarytool submit`
accepts a `.dmg` and `stapler` staples one — that is the ordinary shape for a
direct download. So the options are an extra submit-and-staple step after the
bundle, or shipping the `.app` in a zip instead. Tracked as #108 rather than left as a
paragraph here.

Until then: **before announcing a release, download the `.dmg` on a Mac that
has never seen this project and open it.** Same rule as installing the release
APK rather than the debug one.

## 5. The secrets, in one place

`release.yml` reads these. With none of them set the job builds an unsigned
app and says so, which is the state today and is not an error.

| Secret | Holds |
|---|---|
| `APPLE_CERTIFICATE` | the Developer ID `.p12`, base64-encoded |
| `APPLE_CERTIFICATE_PASSWORD` | the password set when exporting it |
| `APPLE_API_KEY` | the Key ID |
| `APPLE_API_ISSUER` | the Issuer UUID |
| `APPLE_API_KEY_P8` | the `AuthKey_<KEYID>.p8`, base64-encoded |

The last is named `_P8` rather than `_PATH` deliberately: it holds the file's
*contents*, and the job turns it into a path. A secret called
`APPLE_API_KEY_PATH` holding a path would be a path to nothing.

```bash
base64 -i clispeak.p12 | pbcopy              # APPLE_CERTIFICATE
base64 -i AuthKey_XXXXXXXXXX.p8 | pbcopy      # APPLE_API_KEY_P8
```

**The macOS signing key does go into CI now, and decision 57 said it should
not.** That decision was about a *self-signed* certificate, which would have
put a private key in a job that runs several hundred build scripts in exchange
for changing nothing a downloader sees. A Developer ID certificate changes
everything a downloader sees, so the trade is different — the same reasoning
as decision 60 reached for Android, and the same one-job containment applies:
written, used, removed.

## 6. Checking it worked

On the artefact, not on the build log.

```bash
codesign -dv --verbose=4 clispeak.app 2>&1 | grep -E 'Authority|TeamIdentifier'
xcrun stapler validate clispeak.app
spctl -a -vvv -t exec clispeak.app
```

- `Authority=Developer ID Application: …` — signed with the right kind of
  certificate. `Signature=adhoc` means the identity never reached `codesign`.
- `The validate action worked!` — the ticket is attached, so this works
  offline.
- `source=Notarized Developer ID` — what Gatekeeper will conclude. `source=No
  matching rule` or a rejection means it would refuse.

**`spctl` has a third answer and it is not a verdict.** Against the ad-hoc
bundle today it says:

```
clispeak.app: code has no resources but signature indicates they must be present
```

That is neither `accepted` nor `rejected`. It means the signature could not be
evaluated at all, so read it as a statement about the artefact rather than
about Gatekeeper's opinion of it — the tool is not broken.

On the disk image, ask the question a downloader's Finder asks:

```bash
spctl -a -vvv -t open --context context:primary-signature clispeak_0.1.0_aarch64.dmg
xcrun stapler validate clispeak_0.1.0_aarch64.dmg
```

**This page previously said `-t exec` on a `.dmg` gives an answer that does
not mean what the reader thinks. Measured on a signed image, that is wrong:**
both forms give the same, correct verdict.

```console
-t open --context context:primary-signature   rejected / source=Unnotarized Developer ID
-t exec                                       rejected / source=Unnotarized Developer ID
```

`-t open` is still the more precise question to ask about a disk image, and it
is what this page recommends. But the practical warning attached to it was
reasoning about a signed image at a time when no signed image existed, and it
did not survive one being built. Corrected rather than quietly dropped,
because the wrong version was specific enough to act on.

Signed and un-notarised is a **named** rejection, not a silent one — Gatekeeper
says `source=Unnotarized Developer ID`, and the app inside a mounted copy
assesses the same way. What a downloader sees once the `.app` carries a
stapled ticket and the `.dmg` does not is still unmeasured, and is #108.

The release job runs the first two of these itself and fails the build if
credentials were supplied and the artefact came out without them.
