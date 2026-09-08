# 60. The Android key goes in CI, and the macOS one does not

**Status:** Accepted.

**Chosen:** a release keystore held in four repository secrets, written to
`gen/android/app/keystore.properties` by the release job, read by
`build.gradle.kts` through the same file a local build reads, verified with
`apksigner` and deleted afterwards. With no secrets set the job builds and
says the APK is unsigned, which is today and is not an error. The recipe for
creating the key is `docs/signing-android.md` and it is Patrick's to run.

**Why this contradicts decision 57.** That decision kept a code-signing key
out of CI on the grounds that the job runs several hundred build scripts that
can read the environment. Every word of that still applies here — and the key
goes in anyway, because the two artefacts fail differently when unsigned. An
unsigned Mac app runs after a Gatekeeper warning; an unsigned APK does not
install at all. So on macOS, refusing the key costs a warning; on Android it
costs the entire artefact. The risk was not judged smaller, it was judged
worth paying, and the job narrows it where it can: the keystore is written and
removed inside one job, never checked out, and the release workflow already
holds `contents: write` on the draft step alone (#55).

**Why the key is worse to lose than any other credential here.** An Android
signing key cannot be rotated. Play and the on-device installer both identify
an app by the certificate that signed it, so a replacement key is a different
app: no update path, every install lost. A leaked token is revoked; a leaked
keystore is permanent, and the only backup that helps is one made before it is
needed.

**Why a properties file and not environment variables.** Gradle would read
either. A file means the local path and the CI path are the same path, so a
mistake in one is reproducible in the other — and the mistake this shape
already caught is worth the whole design: `file()` in a Gradle script resolves
against the *module* directory, not the project root. A keystore file one
level up is read as absent, `signingConfigs` is never created, and the release
build comes out unsigned. `signingReport` reports that as `Config: null`, not
as a missing file. Measured with a throwaway keystore before the document was
written, because the document had it in the wrong place.

**Why the job reads the signature back.** Same reasoning as decision 57, and
the same failure shape: a build configured to sign that silently did not is
indistinguishable from a signed one unless something asks. `apksigner verify
--print-certs` asks. When no secrets are set the step instead asserts the
artefact's name still contains `unsigned`, so "we meant not to sign" and "the
secrets did not arrive" cannot be confused with each other.

**Signing keys are ignored from the repository root**, not from Tauri's
generated ignore file under `gen/android/`, which already covered
`keystore.properties` and covered the keystore itself with nothing at all. A
generated file can be regenerated; a rule that can be silently taken away is
not a rule. `*.jks`, `*.keystore` and `*.p12`.

**Costs.** Four secrets to set up, a keystore Patrick has to back up himself,
and a signing path exercised only on tags — so a mistake in it surfaces at
release time. The `versionCode` is still Tauri's, which Play requires to
increase on every upload; nothing enforces that yet.
