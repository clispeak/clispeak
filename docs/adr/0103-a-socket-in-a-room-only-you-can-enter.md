# 103. A socket in a room only you can enter

**Status:** Accepted.

Any local user could take the socket name before the node did. Decision 76
closed the disclosure half — a squatter cannot drive the node or receive what
was meant for it — and #160 stopped the node blaming itself when one did. What
was left was the operating system enforcing ownership, which needs a real file
in a directory nobody else can open (#128).

**`CLISPEAK_SOCKET` keeps meaning a name**, on Patrick's decision, so
`CLAUDE.md`, `docs/cli.md` and the README stay true. What moved is where the
name is resolved: `$XDG_RUNTIME_DIR/clispeak/` where there is one — per-user,
`0700`, on tmpfs, cleaned at logout — and the per-user temporary directory
otherwise, which is what macOS has.

**Creating the directory is not the fix. Checking it is.**
`create_dir_private` sets `0700` on a directory it creates and returns `Ok`
for one that already exists, whatever mode somebody else gave it. On a machine
where the base falls back to `/tmp` — Linux with no `$XDG_RUNTIME_DIR`, which
is a container or a session-less service — that is the original attack
arriving *through the fix for it*, on exactly the machines least able to
notice, while every document said the socket was protected.

So the node verifies before binding: a real directory rather than a symlink to
one, owned by whoever is running, and closed to group and other. Anything else
is a refusal that names the mode. Demonstrated rather than argued —
`chmod 0777` on the directory produces:

```
Error: the socket directory /run/user/1000/clispeak is mode 0777, which lets
others in. It has to be a directory owned by you that nobody else can enter,
or another local user could take the socket name before this node does.
```

**Ownership is established without `unsafe`.** `libc::getuid` needs an
`unsafe` block and this crate *forbids* unsafe, which is worth more than the
convenience. A file we have just created is ours by construction, so its owner
is the answer — and creating it doubles as the write test, since a directory
owned by someone else and mode `0700` refuses the probe.

**One resolver, three callers.** The node, the liveness probe and the CLI each
spelled out where the socket was, which is three chances to disagree — and a
disagreement here does not look like a bug, it looks like no node running at
all. There is now one function, and the CLI mirrors it by hand for the reason
it mirrors the frame format: that binary depends on two crates and nothing
else, and its 3ms startup is the premise of the thin-client design.

**The CLI does not mirror the checks**, deliberately. Ownership and mode are
assertions the *listener* makes about where it is about to bind. A caller only
has to look in the same place, and a caller that refused to look would be
refusing to talk to a node that had already satisfied them.

**A name with a separator is now refused** rather than reinterpreted. It was a
note while the platform decided the location and there was often no file
anywhere (#43); now it would silently create a directory somewhere nobody
expects.

**Windows is not fixed and says so in the code.** A named pipe has no
directory to live in and needs a security descriptor on the listener, which
`ListenerOptions` supports and nobody has written. Named at the point the pipe
is created rather than only in an issue, because a gap recorded only in a
tracker is a gap nobody reading the function will see.

**Costs.** Six declared portability exceptions, all of the same shape: a
named pipe and a filesystem socket cannot be reached by one spelling. The
budget question changes from decision 35's 83-byte *name* to a 104-byte
*path*, which decision 95 measured before this was built — the right order,
because a budget that did not fit would have changed the design rather than
the details.
