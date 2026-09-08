# 43. The keyring is asked once per process

**Status:** Accepted.

`describe()` called `get_secret()` again on every start, purely to print where
the key came from. On macOS every call is a keychain prompt, so starting a node
asked twice for one secret, compounding #29. Issue #83.

One private `probe` is now the only thing that touches the keyring, and one
`ask` caches it. Saving updates the cache rather than invalidating it, so
`describe` cannot contradict a write that just succeeded.

**A locked keyring is not an empty one.** Every error was collapsed to "no key"
through `.ok()`, so a locked keychain and a device with no identity produced the
same word: `file`. The marker file correctly refused to mint a new identity, so
the node stopped — while the line above it said the key was in a file. Only
`keyring::Error::NoEntry` now means empty; anything else is `Unreadable` and
keeps the reason, so `describe` can name the keychain as the thing to unlock.

**Cost.** A keyring that becomes readable during the life of a process is not
noticed until restart. That is the right trade against a second prompt on every
start, and restarting is what the message tells you to do anyway.
