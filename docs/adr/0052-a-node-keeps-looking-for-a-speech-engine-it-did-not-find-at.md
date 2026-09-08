# 52. A node keeps looking for a speech engine it did not find at startup

**Status:** Accepted.

Discovery ran once and the answer was kept for the life of the process. The
first-run path was therefore: open the app, be told Piper is not installed,
run `cargo xtask piper`, and be told Piper is not installed — by a node whose
statement had stopped being true several seconds earlier. The README said to
install it and did not say to restart, because whoever wrote that line did not
know it mattered. Issue #84.

**A wrapper rather than a re-probe at every call.** `Rediscovering` holds the
reason there is nothing yet and re-runs the probe at most every two seconds
until something answers, then keeps it. Two seconds is far below how long it
takes a person to install something and try again, and far above the cost of
the probe, which is a handful of `stat` calls against paths that are usually
absent. Once an engine is found it is not looked for again: a working Piper
does not need finding twice, and probing forever would walk the filesystem on
every utterance for no reason. `stop` deliberately does not probe — an engine
starting because something was cancelled would be absurd.

**It replaces the silent fallback, not the working one.** Where Piper is found
at startup nothing changes. What changed is the case where nothing was found,
which used to be permanent.

**One case is still not covered, and is worth naming.** On Linux with
espeak-ng present, Piper missing gives you espeak, and installing Piper later
does not upgrade you until restart. That is a different complaint from the one
in the issue — you are told "espeak (fallback)", which is true — and fixing it
means teaching the wrapper to hold a working engine while still looking for a
better one. Not done, because "my device speaks, in the wrong voice, until I
restart it" is a smaller problem than "my device says the thing I just
installed is not installed".
