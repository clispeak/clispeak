# 63. The five-target verdict is bought once, at the point that asks for it

**Status:** Accepted.

**Chosen:** the build matrix skips draft pull requests. The Linux job — fmt,
clippy, the portability gate and the whole test suite — still runs on every
push, draft or not. Open a pull request as a draft, push as often as you like,
and mark it ready when you want the answer. `ready_for_review` is added to the
trigger types, without which the whole thing is a trap: the expensive jobs
skip while a pull request is a draft, marking it ready fires no event, and it
sits with four skipped checks forever.

**The rule this project opens with does not move.** Nothing merges without
building on all five targets. That is a rule about *merging*, and we were
paying for the verdict on every push instead of on the one that asks for it.

**Measured, because the answer was not where either of us would have looked.**
One day — 3 September, 85 runs, 417 jobs, durations from the jobs API, each
job rounded up to a whole minute and weighted at the private-repository
multipliers:

| | billed Linux-equivalent minutes |
|---|---|
| total | 3,649 |
| pushes to `main` | 2,445 (67%) |
| pull requests | 1,204 (33%) |
| macOS runner | 2,150 (59%) |
| Ubuntu runners | 769 |
| Windows runner | 730 |

A standard plan includes 3,000 a *month*. One day exceeded it.

Two readings matter. **macOS is 59% of the spend on 20% of the wall-clock** —
the job does about 1.3 minutes of real work and bills at ten times a Linux
minute, so making it faster cannot help and only running it fewer times can.
And **two thirds of the spend was `main`**, which is not an argument against
decision 53 — cancelling runs there is how a red commit landed with no verdict
— but an artefact of a week of direct pushes during the audit. Now that
everything goes through a pull request, `main` runs once per merge and that
number falls without a change.

**Why the cheap job still runs on drafts.** It costs one Linux minute and it
is where nearly every mistake is actually caught. Skipping it would trade the
money for exactly the feedback loop that makes a draft worth pushing to, which
is the whole mechanism this depends on.

**Costs, and one deliberate omission.** A pull request opened ready by habit
pays the old price, so this relies on a convention rather than enforcing one —
it is in `CLAUDE.md`. Path filtering would be the next win and is deliberately
not done: a pull request touching only `app/src/**` or `docs/**` does not need
four Rust cross-builds, but a filter that wrongly decides *not* to build is
invisible, and that is the failure shape this repository has paid for more
than any other. A build that never ran reports nothing at all. The draft
change gets most of the money with none of that risk (#101).
