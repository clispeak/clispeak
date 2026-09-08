# 104. What 1.0 is, and what it deliberately is not

**Status:** Accepted.

Patrick, 4 September 2026. Recorded because it reorders half the open issues
and because a scope decision that lives only in a conversation gets
re-litigated by whoever touches it next.

**1.0 is Linux, Windows and Android, downloaded from a page.** macOS is
finished and comes along; iOS says "coming soon" and ships nothing.

| platform | 1.0 |
|---|---|
| Linux | Flatpak from the page |
| Windows | installer from the page |
| Android | **APK from the page, not Google Play** |
| macOS | signed and notarised `.dmg` |
| iOS | **"coming soon"** — keeps building, keeps being tested, ships nothing |

**Why not the Play Store for 1.0.** A store listing is an account, a review,
a privacy policy, a data-safety declaration and a target-SDK treadmill — and,
for an individual account, twelve testers for fourteen days before production.
None of that makes the software better, and all of it happens before anyone
can use it. The order is: make a release that works, let friends install it,
then take the routes that need paperwork.

**What it costs, and this is the uncomfortable half.** Play App Signing was
the thing that made the Android key reversible: Google would hold the real one
and ours would be a resettable upload key. Choosing a download link gives that
up. The keystore made for 1.0 **is** the app's identity to every device that
installs it, for ever, and losing it strands every user with an uninstall that
takes their identity, spaces and pairings with it. #31 now carries that in as
many words.

**And the later choice is constrained by the key made now.** When Play does
happen, either that key is handed to Play App Signing — so existing
sideloaded installs can still update — or Google generates a fresh one and
everyone who downloaded from the page has to uninstall and reinstall.

**Why iOS keeps building while shipping nothing.** It would be cheaper today
to stop. It is the platform where things break *silently*: it compiled for
months while linking the one binary that could not reach the network (#131),
and it is suspended in the background in a way no other platform is (#137). A
platform nobody builds is a platform that has quietly stopped working, and the
bill arrives when somebody tries to pick it up. Deferring distribution is
cheap; deferring the build is not.

**What this reprioritises.** #30, the Windows installer, becomes 1.0-critical
— it is the only platform nobody has ever run. #45, checking that a release
APK launches, matters more now that an APK download *is* the distribution.
#25 gets a definite shape rather than an open question. #138, TestFlight, and
Play for Android both move to after 1.0.
