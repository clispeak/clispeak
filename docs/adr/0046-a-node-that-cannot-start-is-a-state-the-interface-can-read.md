# 46. A node that cannot start is a state the interface can read

**Status:** Accepted.

Three failures in the app shell, all the same shape: something went wrong, the
reason existed, and the only place it went was stderr — which the code's own
comment already noted is "nowhere at all" for an app launched from Finder.
Issue #72.

**A second node no longer gets as far as the network.** `serve()` refused to
bind a socket another node held, and the app printed that and carried on. By
then `Transport::bind` had put a second iroh endpoint online *under this
device's secret key*, presence checks were running, and every command worked
against the same roster and history files. The window looked healthy and
reached nobody.

Reproduced on a Mac with a `voicecastd` already running: two live endpoints on
different UDP ports sharing one `identity.key`. It needs no second copy of the
app — the daemon and the app are different programs that both want the socket —
which is why LaunchServices refusing a second instance of the same bundle does
not make this moot.

`ipc::node_is_listening` is now asked first, before the key store and long
before the transport. Connecting is the test rather than the presence of a
name, for the same reason `bind_ipc` connects: only a refused connection proves
nothing is listening. Verified: zero UDP sockets bound, and the keychain never
opened — a doomed launch no longer costs a prompt either.

**Why the teardown is kept as well.** The check leaves a race — another node
can claim the socket between the check and `serve()`. `Node::close` takes the
endpoint off the network in that case, where `shutdown` only ended the speaking
thread. iroh documents that the UDP socket itself survives until the last
`Endpoint` clone drops, and the presence-check task holds one for the life of
the process, so the socket lingers; the endpoint is closed, which is what
decides whether a peer is directed to it.

**"Starting…" for ever.** `AppState` is registered only once a node exists, so
every command failed with "state not managed" until then. The interface could
only read that as "still coming up", and read it that way for as long as the
window was open. A locked keyring or an unwritable config directory looked
exactly like a node two seconds from ready.

`StartupState` is now registered before anything can fail and holds one of
three answers — starting, running, failed with a reason. `status_of` is split
from the command so those three can be tested, since a `tauri::State` cannot be
built outside a running app, which is part of why this was never caught.

**Cost.** One more managed state and one more field on the wire to the
frontend. The failure banner is a second red panel on the home screen,
deliberately distinct from the engine one: that says a running device cannot
speak, this says there is no device.
