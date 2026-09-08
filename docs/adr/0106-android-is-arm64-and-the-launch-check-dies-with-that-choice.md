# 106. Android is arm64, and the launch check dies with that choice

**Status:** Accepted.

Patrick, 4 September 2026: *"I don't think we need to worry about supporting
Intel 64 Android apps or the older 32-bit stuff. I mean that's ancient and no
point using an emulator cuz we got builds for everything else."*

The release APK stays `arm64-v8a` only, which is what `--target aarch64` has
been producing all along. What changed is that it is now a decision rather
than a default nobody had looked at — decision 104 made the APK the
distribution, so the question "which phones does this file install on" stopped
being an internal detail.

**What it excludes:** x86_64 Android emulators, and 32-bit-only phones, which
means roughly pre-2019 budget devices. Carrying the other ABIs would take the
download from about 25MB to about 85MB to serve people who are not going to
install this.

**What it costs is one thing, and it is not size.** #45 wanted CI to check
that the release APK *launches* — the check that would have caught #41, where
the APK installed perfectly and died on its first frame because R8 had deleted
the Kotlin the engine calls. CI runs on x86_64. An x86_64 emulator cannot run
an arm64-only APK. So the check needs either an ABI we have just declined or
an arm runner we are not buying, and the issue is closed as impossible rather
than deferred as expensive.

There is no cheaper version hiding underneath it. An *install* check would
have passed #41.

**So the thing that replaces it is a person**, and `CLAUDE.md` already
requires it: install the release artefact and open it before a release, on
real hardware, and say which hardware. That line was written after #41 and has
been reading like belt-and-braces. It is now the only thing standing between
us and that bug happening twice.

**And it leaves the download page owing a sentence.** The file is called
`clispeak-android.apk` and names no architecture. An install that fails on an
emulator says nothing about ABIs, so the page has to — alongside the three
sentences the other platforms already need, for Gatekeeper, SmartScreen, and
Android's "install unknown apps" prompt.
