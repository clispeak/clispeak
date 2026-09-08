# 107. The Flatpak installs a command-line tool it cannot be driven by

**Status:** Accepted.

Found on 6 September 2026, by Patrick restarting his laptop.

The Linux release artefact is the Flatpak. It installs `clispeak` into
`~/.local/bin` — deliberately, because entering the sandbox costs ~86ms per
call against the tool's own 3ms and an agent calls it repeatedly (decision 77).
**That tool then cannot talk to the app that installed it.**

The node writes an IPC token and the CLI has to prove it knows it (decision
76). Inside a Flatpak, `XDG_CONFIG_HOME` points at `~/.var/app/<id>/config`,
so the token lands there. The CLI runs on the host and reads
`~/.config/clispeak`. Same machine, same app, two directories, and the
handshake fails on every call.

**What made it cost an hour is that nothing pointed at the file.** The socket
needs no grant at all: `--share=network` puts the sandbox in the host's
network namespace, and a namespaced name on Linux is an abstract socket, which
lives there. So the CLI *connects*. It is then refused, and the error says

> something else on this machine holds the socket named "clispeak.sock", and
> it is not a clispeak node this user started

which is true of what the node observed and false about what happened. It sent
me looking for a squatting process, then to start a second node — which had a
stale roster from before the rename and briefly replaced a working device with
a stranger. One unwritten file, and every symptom pointed somewhere else.

**The fix is one grant and one extra write.** `install_token` still writes to
the node's own config directory; the node now also publishes a copy to the
host's, when it is not already running on the host, and the manifest grants
`--filesystem=xdg-config/clispeak:create` — one directory, ours, no
executables in it, and narrower than the two grants already there.

**The part worth writing down is the variable that would have shipped a fix
that worked only here.** The obvious way to find the host's config directory
from inside the sandbox is `HOST_XDG_CONFIG_HOME`, which Flatpak exports. It
exports it **only when the host had `XDG_CONFIG_HOME` set to something** — and
most machines leave it alone. This one does not, so the variable is present
here and absent almost everywhere else. Measured rather than assumed: set in
the sandbox, and gone the moment the host variable is unset.

A fix reading only that variable passes every test anyone would run, on the
one machine anyone would run them on, and does nothing at all for users. It is
the same shape as `/etc/hostname` and `base64 -d` in `CLAUDE.md`: not
platform-shaped, no `cfg` to make it visible, and it fails by being silent.
So the resolution falls back to `$HOME/.config`, which is the XDG default and
which the sandbox can see, and the four cases are unit tested.

**And the failure is now said out loud.** If the grant is ever missing — a
Flathub build that refuses it, an override someone removed — the node prints
that its own CLI will not work and names the grant, instead of leaving a
working app and a command that lies about why it failed.

**What this does not fix.** Two installs on one machine still keep two
rosters. They share a device identity, because that lives in the system
keyring, so whichever binds the socket first decides which address book the
CLI sees. That is a separate problem and it is filed rather than fixed here.
