# 120. The Windows pipe carries access rules, and CI is what checks them

**Status:** Accepted.

**Chosen:** the named pipe is created with a protected DACL —
`D:P(A;;GA;;;OW)(A;;GA;;;SY)(A;;GA;;;BA)` — and the `ipc` tests run on a
Windows runner so that DACL is executed before it ships.

**Why.** #128 was the half of #54 that decision 76 deliberately left. Decision
103 closed the Unix half by putting the socket inside a `0700` directory of
this device's own; Windows has no directory to put a pipe in, so the same
boundary has to be spelled as an access-control list on the listener.

`OW` is the Owner Rights SID, which Windows evaluates against whoever owns the
object — for a pipe this process created, the account running it. That is what
makes it work without looking up a SID, which would mean Win32 calls in a
workspace that *forbids* `unsafe`. `SY` and `BA` are there because LOCAL
SYSTEM can reach any object and an administrator can take ownership of
anything, so refusing them buys nothing and costs an app that will not talk to
its own CLI across an elevation boundary. Everyone, Authenticated Users and
Interactive are absent, which is the entire point.

**What it does not do.** It does not stop squatting, and nothing can. The
Windows pipe namespace is global and first come; an access-control list on our
listener says nothing about somebody else's. #128's headline complaint is not
fixable in the shape the issue imagines — what was fixable was the consequence
it named, "the reason on screen is wrong", and `who_is_listening` fixed that
in decision 103 by identifying a stranger as a stranger and offering
`CLISPEAK_SOCKET` as the way round.

**The part that is not the DACL.** This is a security control on the one
desktop platform none of the people writing it run, and the failure it risks
is not a list too loose but one too *strict* — an app that cannot open its own
pipe, arriving as "the app is broken" days later with nothing pointing at a
constant. `cargo check` cannot reach this code from Linux at all: `ring` needs
an MSVC toolchain, so the cross-check fails before type-checking starts. So
the change is not trusted; it is executed, by `cargo test -p clispeak-core
--lib "ipc::"` on the Windows matrix job.

**Stated plainly: that test checks one half.** It proves the creating account
can open the pipe. Proving a *different* account is refused needs a second
logon session, which no runner has. The exclusion rests on the SDDL being what
it says it is, which is why every term is written out in the constant's doc
comment rather than left as an opaque string for the next person to decode.

**What it costs.** One Windows test invocation, scoped to a module, on a
runner that bills at twice Linux. Against a security control that would
otherwise be verified by a person installing a release, that is not a close
call — and it is the first test this project has ever executed on a platform
other than Linux.
