# 19. The Flatpak bundles Piper and ships its own indicator library

**Status:** Accepted.

**Decision.** The Flatpak carries a Piper binary and one voice in
`/app/share/voicecast`, and builds libayatana-appindicator as a module.

**Why.** The GNOME runtime has no speech synthesiser at all — not even
espeak-ng — so a sandboxed install without a bundled engine starts up mute,
and the "using a fallback voice" state the interface reports would have
nothing to fall back to. Bundling costs about 85MB and makes the app work the
moment it is installed.

The runtime also ships no appindicator library, which the tray needs. That
matters more than a missing icon: closing the window deliberately hides it so
the node keeps running, so with no tray there is no way back to the window
except `voicecast show` from a terminal.

Shipping the library is necessary but not sufficient. An indicator claims its
own bus name and then calls the watcher, and the sandbox forbids both by
default, so the manifest also grants `--own-name=org.kde.StatusNotifierItem-2-1`
(the id is the process id, which inside a sandbox is always 2) and
`--talk-name=org.kde.StatusNotifierWatcher`. Without those the icon silently
never appears, with no error anywhere.

The app installs the command-line tool onto the host PATH itself, on first
launch, rather than offering it behind a button. The tool is the whole point
of the node — it is how an agent reaches it — so leaving it uninstalled until
someone finds a control means `voicecast` is simply missing from the PATH.
The same pass rewrites the copy whenever its bytes differ from the bundled
one, which is what keeps a Flatpak update from leaving a stale CLI behind.

**Consequences.** Piper is looked up in several roots now — the user data
directory first, then `/app/share/voicecast`, then `/usr/share/voicecast` —
so a user-installed voice still wins over the bundled one. A tray that fails
anyway is caught rather than fatal, including panics, because the tray crate
panics on a missing library instead of returning an error.

A Flatpak install has its own data directory, so it does not inherit the
space of a non-Flatpak install on the same machine. The device identity does
carry over, because that lives in the system keyring. Devices therefore have
to rejoin after switching to the packaged build.
