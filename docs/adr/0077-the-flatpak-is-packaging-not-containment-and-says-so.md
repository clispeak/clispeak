# 77. The Flatpak is packaging, not containment, and says so

**Status:** Accepted.

**Chosen:** keep both filesystem grants, and state plainly in the manifest,
the README and here that they make the sandbox nominal. Close the
`.gitignore` gaps around Flatpak build output, and verify the Gradle wrapper
in CI. The app identifier remains Patrick's to pick.

**The sandbox does not contain anything.** `--filesystem=~/.local/bin:create`
grants write access to a directory that is on the user's PATH *ahead of*
`/usr/bin` on every mainstream distribution — so it is write access to the
next command they type. `--filesystem=~/.claude:create` grants write access to
Claude Code's hooks, which are commands it runs on the user's behalf. Either
one alone is an escape. Both were added deliberately, each with a comment
explaining why, and neither comment mentioned what it costs (#82).

**They stay, because the alternative fails in the way this project keeps
paying for.** An unshared write inside a Flatpak *appears to succeed* and
never reaches the host. An app that offers to install the CLI, reports
success, and puts the file nowhere is the same shape as `isMinifyEnabled`
deleting the Kotlin and `file()` resolving against the wrong directory. Given
the choice between a sandbox that is real and an install that silently lies,
the install wins — and the sandbox stops being described as one.

**So the decision is the sentence, not the grants.** "Install it because it is
convenient, not because it is contained." Decision 19 chose to bundle Piper
and ship an indicator library; it did not say what the manifest gives away,
because at the time nobody had added that up.

**Flathub refuses `~/.local/bin` outright**, so publishing there needs a
different answer rather than a smaller version of this one. That is #23's
problem and it is named now rather than found during a submission.

**`gradle-wrapper.jar` is a binary in the tree that every Gradle invocation
executes**, it arrived with Tauri's `android init`, and nobody has ever
verified it. Reviewing a jar in a diff is not a thing anyone does, so CI now
checks it against the checksums Gradle publishes. A tampered wrapper runs
arbitrary code on the runner and on any machine that builds the app.

**The `.gitignore` gaps were larger than they looked.**
`packaging/flatpak/build-dir/` is 160MB and was invisible only because
`flatpak build-init` writes its own `.gitignore` containing `*` inside it — an
ignore rule this repository neither owns nor asked for, and one that would
stop protecting us the moment the directory was created another way.
`repo/` and `*.flatpak`, which `release.yml` creates, were covered by nothing.

**Costs.** One more CI step on the Android job, which is seconds. And the
honest sentence about the sandbox is a worse thing to read on a download page
than silence would have been, which is the point of writing it.

**Left open deliberately:** the identifier split — `org.voicecast.App` in the
Flatpak against `com.voicecast.app` in `tauri.conf.json` and
`build.gradle.kts`. It cannot be changed after publication without users
losing their settings, Flathub wants proof of control of `voicecast.org` for
the first form, and a mismatch between the desktop id and the window's app id
can cost the dock icon on GNOME. That is Patrick's call and it gates #23.
