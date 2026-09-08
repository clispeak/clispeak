# 65. Notarisation is a second thing, and the release says whether it happened

**Status:** Accepted.

**Chosen:** the release job stages an App Store Connect API key out of a
secret, passes the notarisation variables to `tauri build`, removes the key on
every path out of the job, and then asks the *artefact* whether it carries a
stapled ticket. `docs/signing.md` gains the paid half — enrolment, which
certificate, both credential routes, and what to check.

**Signing and notarisation are two things and Gatekeeper wants both.** A
Developer ID signature on its own is still refused. There is no half-way state
worth stopping at, which is why this lands as one change rather than as
"signing now, notarisation later".

**The API key route is documented as primary**, over an app-specific
password. Both work. The key is scoped to the team and revocable on its own;
an app-specific password is a credential on the Apple ID that owns the
membership, so losing one costs the account rather than the key. Read out of
`tauri-bundler` 2.9.4 rather than out of documentation, which corrected two
things a reading of the compiled CLI's strings had suggested.

**The bundler only ever hands `notarytool` a path**, never the key's
contents — `notarize_auth` constructs `ApiKey::Path` on both branches and
nothing constructs `ApiKey::Raw`. So the file must exist before the build and
nothing places it. On a Mac the bundler searches `./private_keys`,
`~/private_keys`, `~/.private_keys` and `~/.appstoreconnect/private_keys`, so
locally two variables suffice; a runner has no such directory, so the job
writes the file and says where. The secret is `APPLE_API_KEY_P8`, not
`..._PATH`, because it holds the file and a secret named for a path would be a
path to nothing.

**Why the check exists.** Missing notarisation credentials make the bundler
log a *warning* and carry on, producing a signed, un-notarised app. A warning
in three hundred lines of build log is not a report, and the result is a
release that is correctly signed, cleanly built, and refused on every machine
that downloads it. `xcrun stapler validate` asks the artefact instead. It
fails the build for the one combination that is a lie — credentials supplied,
no ticket — and otherwise says which state it is in. The same shape as
decisions 57 and 60, for the third time in one file.

**This puts a signing key in CI, which decision 57 declined.** That decision
concerned a *self-signed* certificate, which would have bought a downloader
nothing while putting a private key in a job running several hundred build
scripts. A Developer ID certificate buys the downloader everything, so the
trade is a different trade — the same reasoning decision 60 reached for
Android, and the same containment: written, used, removed.

**`base64 --decode`, not `-d`.** macOS spells the short flag `-D`. The Android
job's `-d` is correct only because it runs on Linux, and the same line on the
macOS runner would write an empty key and fail later somewhere else. Added to
the divergence table in `CLAUDE.md`, which is the fifth entry there that is
not platform-*shaped* and would not have been caught by a `cfg`.

**All three variables locally, not two.** The bundler's search path for
`AuthKey_<KEYID>.p8` is real, and building the instruction on it would be
wrong: when the search misses, notarisation is skipped, a warning goes into
the log and a signed but un-notarised app comes out. That is the exact
degradation this decision adds a check for, reintroduced in the setup
instructions. Naming the path makes a missing key loud, and makes the Mac and
CI one story rather than two.

**Costs, and the gap that is now an issue.** A paid membership, an enrolment
that is not instant, and six secrets. Apple limits a team to five Developer ID
Application certificates and revoking one invalidates every un-notarised app
signed with it. And Tauri notarises and staples the `.app` but only *signs*
the `.dmg` — the disk image is never submitted, so no ticket for it exists
stapled or in Apple's database. Measured on a Mac: quarantine propagates from
the image to the app copied out of it, so both are assessed; the app carries
its ticket and the image carries nothing. That is narrower than "untested" and
it is fixable — `notarytool` accepts a `.dmg` and `stapler` staples one — so
it is #108 rather than a paragraph. Until a Developer ID certificate exists
none of it is testable, and the document says to open the `.dmg` on a fresh Mac
before announcing a release, in the same terms as installing the release APK
rather than the debug one.
