# 72. The Piper payload signs the way notarisation requires

**Status:** Accepted.

**Chosen:** `xtask`'s `sign()` passes `--options runtime` and a secure
timestamp, rather than `--timestamp=none` and no hardened runtime.

**Apple would have rejected the first notarisation attempt.** `tauri-bundler`
signs nested code only in `MacOS`, `Frameworks`, `Plugins`, `Helpers`,
`XPCServices` and `Libraries`. `Resources` is not on that list, and the Piper
payload is declared `"resources": ["speech/**/*"]`, so it lands in
`Contents/Resources/speech/` and Tauri never touches it. This function is the
only thing that signs those four Mach-O files.

Measured on a Developer ID build before the fix:

| | runtime | timestamp |
|---|---|---|
| `MacOS/voicecast` | yes | yes |
| `MacOS/voicecast-app` | yes | yes |
| `Resources/speech/piper/piper` | **no** | **no** |
| `…/libonnxruntime.1.14.1.dylib` | **no** | **no** |
| `…/libpiper_phonemize.1.dylib` | **no** | **no** |
| `…/libespeak-ng.1.dylib` | **no** | **no** |

Apple requires both on every executable it notarises. The identity was
correct throughout — the flags were not, and nothing anywhere said so. The
bundle signs, `codesign --verify --deep --strict` passes, the app runs, and
the rejection arrives from Apple during a release.

**Verified past the flags, because the flags are the easy half.** The hardened
runtime enables library validation: a process may then load only libraries
signed with the same team identifier. Piper loads four dylibs from beside
itself through an `LC_RPATH` this crate writes. All four now carry the same
Developer ID, so it should be satisfied — and "should" has been the wrong word
all day, so it was tested. Installed the rebuilt bundle and spoke a message:
history records it `spoken`, not `NoEngine`. A signature that notarises and an
app that cannot speak would have been a worse outcome than the bug.

**Both flags are conditional, and the second one is not a nicety.** An ad-hoc
signature cannot be timestamped, which is the easy half. The hardened runtime
is the half that nearly shipped a worse bug than the one being fixed.

The runtime enables library validation, which requires a loaded library to
carry the same team identifier as the process loading it. Under a Developer ID
`piper` and its three dylibs share a team and it works — measured. **Under an
ad-hoc signature neither has a team at all, and macOS does not read that as a
match:**

```text
Library not loaded: @rpath/libespeak-ng.1.dylib
Reason: code signature not valid for use in process:
        mapping process and mapped file (non-platform) have different Team IDs
```

Applying it unconditionally would have made the app mute for every build
without a certificate — every other developer's local build, and the artefact
CI produces today, since `release.yml` builds unsigned while no secrets are
set. A strictly wider blast radius than the notarisation rejection it fixes,
and the identical failure the signed path had just been protected from.

**The cost of the condition is real and is the other side of the trade.** A
local ad-hoc build no longer exercises the loader restrictions the shipped one
has, so a hardened-runtime problem can only be found on a signed build. That
is worse coverage, and it is still the right way round: a signed build is
testable on this machine, and a mute app on every unsigned build is not
something to trade for coverage.

**Found by review, not by the test.** The mechanism was reasoned correctly and
then verified in exactly one of the two configurations the change affects — the
one that does not ship. The lead asked which case had actually been run, which
is a different question from whether the reasoning was right.

**Costs.** Signing now needs the network, because a secure timestamp is
fetched from Apple. That is a new way for `xtask bundle` to fail on a plane,
and it is not optional for anything intended to be notarised.

**Credit where it belongs.** Inferred by the lead from reading `app.rs`'s
directory list on a machine that cannot run any of this, filed explicitly as
inference rather than fact, and settled here in two minutes because the
inference named exactly what to look at.
