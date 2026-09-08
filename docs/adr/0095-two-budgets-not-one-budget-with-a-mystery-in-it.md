# 95. Two budgets, not one budget with a mystery in it

**Status:** Accepted.

Decision 35 measured the macOS socket-name limit at 83 bytes bound and 84
refused, against a documented `sun_path` of 104, and said the missing 16 bytes
were "unexplained, probably headroom inside `interprocess`'s own check".
`CLAUDE.md` repeated it. It sat there for weeks reading like a fact with a
hedge on it, and the hedge was doing the work.

**Measured on a Mac on 3 September 2026**, by binding at increasing lengths
until refusal:

| | longest bound | first refused |
|---|---|---|
| namespaced name, `interprocess` 2.4.3 | 83 | 84 |
| filesystem path, `interprocess` 2.4.3 | **104** | 105 |
| raw `AF_UNIX` via Python | 103 | 104 |

**There are two budgets.** 83 is what a *name* costs once the library has
reserved room for the prefix it adds. 104 is what a *path* costs, because
nothing is added to it — the whole of `sun_path`. The 16 bytes were never
headroom; they were the other question's answer, and nobody had asked the
other question.

The raw figure is one byte tighter than `interprocess`'s, which is the
terminator.

**Why it was worth measuring now.** #128's remaining half moves the socket
from a namespaced name to a filesystem path inside a private directory, so it
asks the second question, and decision 35's number is the wrong one for it.
Against 83 the design looked marginal: `$TMPDIR` on that Mac is
`/var/folders/4x/mftwd1y15kd_ww2n9qm0l1kw0000gn/T/`, 49 bytes, and the
candidate socket path is 71. Against the real 104 it clears by 33.

**And `$TMPDIR`'s 49 bytes do not grow with the username — measured, across
37 accounts.** The confidential temporary directory is
`/var/folders/<2>/<30>/T/`, and both components are fixed-width hashes rather
than names. `/var/folders` holds one for every account that has ever had one,
so a single Mac already carried 37 samples: bucket width 2 and hash width 30,
every time, behind usernames from 4 to 21 characters — `root`, `daemon`,
`_mdnsresponder`, `macmini-patrick`, `_windowserver` and thirty more.

This was first written down as *reasoning* from the path format, with a note
asking for a second Mac. It did not need one, and the correction is kept in
view rather than tidied away, because the difference between reading a format
and counting 37 of them is the whole subject of this entry. **What is still
one sample is the machine and the macOS version**, and that hedge stays: the
format could change with a release, and there is one of those here too.

What the 33 bytes actually have to absorb is a longer `CLISPEAK_SOCKET`, which
could reach 46 characters before refusing.

**The lesson, which is this file's usual one wearing a number.** A measurement
answers the question it was taken for. Decision 35's was taken for a name, and
was carried forward as though it were about sockets. The tell was there in
plain sight — an unexplained gap — and an unexplained gap in a measurement is
not a mystery to note, it is a sign that two questions have been rolled into
one.

Measured by voicecast-osx-agent, who also reported that the refusal names its
own cause — *"local socket name length exceeds capacity of sun_path of
sockaddr_un"* — so a path that is too long fails loudly rather than
mysteriously.
