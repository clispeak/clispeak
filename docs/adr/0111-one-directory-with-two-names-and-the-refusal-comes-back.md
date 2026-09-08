# 111. One directory with two names, and the refusal comes back

**Status:** Accepted.

Decision 110 downgraded the identity-conflict check to a warning because it
refused a clean install and nobody could explain why. The explanation arrived
on 6 September 2026, from Patrick reinstalling the Flatpak and reporting that
the laptop still had its old space — which it should not have, after an
uninstall with `--delete-data`.

**There were never two directories.** Flatpak's
`--filesystem=xdg-config/clispeak` bind-mounts the host's directory into the
app's redirected config location, so these are one directory:

```
dev=57 inode=1558614  ~/.config/clispeak
dev=57 inode=1558614  ~/.var/app/org.clispeak.app/config/clispeak
```

Same device, same inode, and **neither is a symlink** — which is why string
comparison and `canonicalize` both report two distinct places. The check was
reporting this device as conflicting with itself, and the refusal it produced
met a clean install before it met a second developer.

It explains three things at once that had looked unrelated: why state appeared
in the host directory while the node named the sandbox, why the laptop kept
its space through an uninstall (the bind-mounted content lives on the host and
`--delete-data` does not reach it), and the refusal itself.

**So the question had to change from "are these paths equal" to "are these
paths the same place".** The kernel answers it with device and inode, which
needs `MetadataExt` — a platform conditional this crate does not get to have.
So it asks the filesystem instead: write a file with a name nothing else would
choose into one directory and look for it in the other. That is the shape
`ipc::private_dir` already uses to learn its own owner without `libc::getuid`,
and for the same reason — the portable *question* is answerable when the
portable *API* is not. The probe is removed afterwards, and a test asserts it.

**And the refusal comes back**, as decision 110 said it would when the
behaviour was understood rather than when the pressure was off.

**What is worth keeping is why the first check passed review.** It had six
tests, and one of them was called `our_own_directory_is_never_among_the_siblings`
— which was exactly the right thing to test and tested the wrong version of
it, comparing paths in a world where one directory can wear two. A test that
names the correct property and asserts it of the wrong quantity is more
dangerous than no test, because it is read as coverage.
