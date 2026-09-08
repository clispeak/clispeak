# 55. The release workflow is hardened before there is anything to steal

**Status:** Accepted.

Today this repository has no secrets, so none of what follows was exploitable.
Issues #29 and #31 will add a signing certificate and an Android keystore, and
#23 will publish what these jobs produce. The time to close these is before
that, not after. Issue #70.

**Least privilege.** `contents: write` was set at the workflow level, so all
four build jobs held a token that can push to any branch and publish releases
while executing roughly seven hundred crates' build scripts, Gradle plugins,
npm lifecycle scripts and `flatpak-builder`. Any of those can read
`GITHUB_TOKEN` out of the environment. Only `draft` publishes anything, so
only `draft` has it now.

**`npm ci`, never the install fallback.** `npm ci || npm install` meant that a
lockfile out of step with `package.json` silently resolved `^` ranges fresh
from the registry — so a tag build could ship whatever Tauri CLI was published
that morning, with nothing recording which, and the lockfile drift that should
have failed the run was what triggered it. Verified that `npm ci` succeeds
against the committed lockfile before removing the fallback, so this does not
close by breaking the build.

**Actions pinned by commit.** Every `uses:` was a mutable tag, and
`dtolnay/rust-toolchain@stable` was a *branch*. `softprops/action-gh-release`
in particular receives the write token and the release assets. Moving a tag is
how the tj-actions/changed-files compromise reached its downstream users. Each
pin keeps its version as a trailing comment, because a bare forty-character
hash tells a reader nothing about what it is.

**Timeouts.** No job had one, so the ceiling was the six-hour default — on a
macOS runner, at ten times the Linux rate. A hung Gradle daemon is not a
hypothetical.

**Not done, and worth naming.** `cargo install cargo-ndk --locked` still has
no `--version`, and the Android jobs take whichever NDK the runner image ships
that week while the README pins r29 for local builds. So nothing records which
NDK built a shipped APK. That wants a decision about which to pin to rather
than freezing today's by accident.
