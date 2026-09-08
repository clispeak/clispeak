# 51. The release job builds the variant that ships, and says what it built

**Status:** Accepted.

`release.yml` passed `--debug` to the Android build from the day it was
written, with a comment explaining that a public APK needs a keystore nobody
has created. The keystore reasoning was sound and the flag was the wrong lever:
`--debug` controls signing *and* minification, and `isMinifyEnabled` is true
only for release. So the only artefact this workflow has ever produced was the
build nobody would ever download, and the first release APK to reach a real
phone died on launch because R8 had deleted the Kotlin the engine calls
(#41, #68). The job that exists to catch that could not have.

It now builds the unsigned release variant. Unsigned is fine and honest —
signing is #31, and a keystore is still a secret nobody has created. What
matters is that R8 runs, so the keep rules are compiled rather than merely
gated by `xtask portability`.

**The Android CI job now compiles `voicecast-app`.** It was excluded, so
"compiles on five" for Android meant the libraries compiled and the crate that
actually ships on a phone did not. A desktop-only call added to its
`cfg(target_os = "android")` paths would pass every job and fail on a tag,
twenty minutes into a release (#69).

**Artefacts carry a `SHA256SUMS`.** Nothing signs them yet, so a checksum is
the only way to tell what you downloaded from what was built — and it matters
more, not less, once #23 serves downloads from somewhere other than the
release page, where a bad upload and a tampered one look identical. It is not
a signature, and the release notes say so rather than letting the file imply
more than it can carry.

**Android stops backing anything up.** The node's key on Android is a plain
file in `files/` rather than in the Android Keystore, and Auto Backup uploads
that directory. A restore onto a second phone produced two devices holding one
identity: peers cannot tell them apart, and revoking one revokes both. Both
`allowBackup="false"` and the two rules files, because the flag alone is
enough today and the rules mean that turning backup on later — for settings
worth keeping — cannot re-export the key by accident. Device-to-device
transfer is excluded alongside cloud backup, since a direct transfer would
clone the key without touching anyone's servers. Issue #57.
