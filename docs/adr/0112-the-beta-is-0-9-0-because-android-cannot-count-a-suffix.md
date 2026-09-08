# 112. The beta is 0.9.0, because Android cannot count a suffix

**Status:** Accepted.

Patrick wants a beta out today, with testers. The obvious version for it is
`1.0.0-beta.1`, and it would have cost him the ability to ship a second one.

**Android's `versionCode` is a single integer, and Tauri derives it from the
semver** as `major × 1,000,000 + minor × 1,000 + patch`. Read out of a real
build: `0.1.0` produced `versionCode=1000`. **A pre-release suffix has no
numeric slot in that formula**, so `1.0.0-beta.1` and `1.0.0-beta.2` both
land on `1000000` — and Android refuses to install an update whose
`versionCode` has not increased. The first beta would install; the second
would be rejected on every tester's phone, with a message about the version
code rather than about the suffix that caused it.

So the beta is **0.9.0**, tagged `v0.9.0`. Version codes run
`1000 → 9000 → 1000000`, rising all the way into 1.0, and the number says
"not 1.0 yet" without needing a suffix to say it.

**It is published as a normal release, not a pre-release**, which is the other
half of the same trap: GitHub resolves `/releases/latest/download/…` to the
newest release that is neither a draft nor a pre-release, and those URLs are
what clispeak.com links to. Marking the beta as a pre-release — the honest
label — would have produced a site with four dead buttons (decision 109's
note, and #187).

**Two things it costs.** The app reports `0.9.0` where a person might expect
to read "beta", so the release notes carry that word instead. And the version
lives in four places rather than one: the workspace `Cargo.toml`,
`tauri.conf.json`, and two internal path dependencies that pin `version =
"0.1.0"` — a path dependency still checks its requirement against the real
crate version, so missing those two fails the build rather than being merely
untidy. There is no gate for this; it was found by grepping before changing
anything.
