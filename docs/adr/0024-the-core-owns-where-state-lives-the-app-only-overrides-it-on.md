# 24. The core owns where state lives; the app only overrides it on mobile

**Status:** Accepted.

**Decision.** `set_config_dir` is called by the app on mobile only. On desktop
the core's own config directory is used, and anything an older build left in
the app's data directory is moved across once.

**Why.** The override existed for a real reason — mobile has no XDG config
directory and `ProjectDirs` returns nothing there — but it was not gated, so
it also applied on desktops, where the directory exists and `voicecastd` was
already using it. One device ended up with two rosters, two histories and two
mute settings, sharing only the identity, because that lives in the keyring.
The symptom is baffling from outside: `voicecast devices` gives different
answers depending on which node happens to be up, with the same device id in
both.

**Why a move rather than a redirect.** Leaving the data where it was and
pointing the daemon at it would mean the daemon computing a path that belongs
to Tauri, which changes when Tauri decides it does. The config directory is
the core's own business, and the core is the thing both processes link.

**Why files already at the destination are set aside, not skipped.** Skipping
would let a stale copy win — the machine that found this had a roster written
by a daemon before the app existed, sitting exactly where the live one was
about to land. Overwriting would destroy it. Renaming it to `.superseded`
keeps both, and being wrong here costs someone every device pairing they have.

**Consequences.** An allowlist of filenames is moved, not the directory: the
app's data directory also holds a web view's caches, which are its own
business. The move is reported at startup rather than done quietly, because it
touches the file that holds every pairing. It is idempotent by construction —
a moved file is gone from the old place — and was verified against a live
three-device roster on Linux before shipping.
