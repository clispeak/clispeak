# 82. The project is called clispeak

**Status:** Accepted.

**Chosen:** the project is renamed from `voicecast` to `clispeak`, under a new
GitHub organisation at `clispeak/clispeak`, with one identifier —
`org.clispeak.app` — used by the Tauri bundle, the Android application, the
Flatpak, the desktop file and the metainfo.

**Why the old name could not stay.** `org.voicecast.App` and
`com.voicecast.app` are both claims to control a domain. `voicecast.org` and
`voicecast.com` are registered to other parties, so neither claim was ours to
make, and Flathub asks for proof of control for the `org.` form. The GitHub
fallback that needs no domain — `io.github.<name>` — was unavailable too,
because `github.com/voicecast` is taken. So every honest identifier was
already gone before the first release.

**One identifier everywhere, which also closes #82's third part.** The
Flatpak said `org.voicecast.App` while the app and the Android build said
`com.voicecast.app`. A desktop file whose name does not match the window's
application id costs the dock icon association on GNOME, and nothing would
have reported it.

**The historical decisions are not rewritten.** Eighty-one entries describe a
project called voicecast, because that is what it was called when they were
written. Editing them to say otherwise would make the record agree with the
present at the cost of being false about the past, which is the exact move
this file exists to prevent.

**A plain desktop install keeps its identity. The Flatpak does not, and that
correction is the useful part of this entry.** `ProjectDirs` derives the
config directory from the project name, so the rename moved it out from under
every install. `migrate_from_previous_name` moves the identity, roster,
spaces, history and policy across on first run, and says which files it moved
rather than doing it quietly — verified against a directory laid out as an
existing device.

That works for `clispeakd` and for an unsandboxed build. **It cannot work for
the Flatpak**, because a Flatpak's configuration lives under
`~/.var/app/<application-id>/`, so the identifier is part of the path. The
renamed app starts in `~/.var/app/org.clispeak.app/` and the migration, running
*inside* that sandbox, looks for a previous-name directory that is also inside
it — never at `~/.var/app/org.voicecast.App/`, where the state actually is.

The identity key survives regardless, because it is in the system keyring and
the manifest grants `--talk-name=org.freedesktop.secrets`. Everything else —
roster, spaces, history, policy, device name — would not, which is the worst
shape available: a node that is still itself, still listed by its peers, and
has forgotten all of them.

So a Flatpak upgrade needs the state directory copied across once, by hand or
by the packaging, and this is written down because "desktop" was assumed to
mean ordinary directories. It is the Android problem again, on a platform
nobody thought of as sandboxed.

An application identifier is the app's name to a phone, so a new one is a new
app with a sandbox the old one cannot reach. Android and iOS re-pair, and the
old app remains installed as a separate icon. That is not fixable and the
release notes owe it plainly.

**The keyring is a second store and `migrate_from` never touched it.** A
keyring item is addressed by service name, so renaming the service orphaned
every desktop identity that lived there. `adopt_previous` reads the old
service, writes under the new one, *then* deletes the old — that order is the
one that cannot lose a key.

**And it is untested, which is said rather than implied.** Exercising it needs
a live keyring, and this repository already has the note explaining why that
cannot be done in a test — `decide` was extracted as a pure function for
exactly this reason. The failure mode is at least loud: `decide` refuses to
mint a fresh identity when the marker says one was in the keyring, so a
desktop that cannot find its key stops with a message rather than silently
becoming a different device.

**A near miss in the testing, worth recording.** The migration test ran
without `CLISPEAK_CONFIG_DIR`, which is the guard that stops a node touching
the real keyring — so it could have adopted and deleted the live desktop
identity. It did not, only because the transient unit had no D-Bus address and
fell back to a file. Saved by an accident of the environment rather than by
the test being written correctly.

**Costs.** Two devices re-pair. Every clone needs its remote updated, though
GitHub redirects. `voicecast` remains installed on the host until removed, as
does the skill at its old path. And the name is now a claim we can actually
back, which is the whole point.
