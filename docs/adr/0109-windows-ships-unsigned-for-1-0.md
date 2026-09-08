# 109. Windows ships unsigned for 1.0

**Status:** Accepted.

Patrick, 6 September 2026, after working through what signing would take:
*"for now we are going to just release unsigned windows builds and kick that
issue into the future - post 1.0 release."* Tracked as #205.

**The thing that makes this a decision rather than an omission** is that the
Apple pattern does not transfer, and finding that out late would be expensive.
Since June 2023 the CA/Browser Forum baseline requires the private key for any
publicly trusted code-signing certificate to sit on FIPS 140-2 Level 2
hardware — a posted USB token, or a cloud HSM. **There is no downloadable
`.pfx`.** So #29's approach for macOS — export a `.p12`, hold it as a secret,
import it into a temporary keychain on the runner — has no Windows twin, and a
USB token cannot be plugged into a GitHub runner at all.

**What it costs, stated plainly because the download page has to say it.**
Every Windows download raises *"Windows protected your PC"*, and the dialog
says "Unknown publisher" — which is precisely the moment a person decides the
software is not safe. That sentence is already on the site and in the README,
and it is the honest mitigation rather than a workaround.

**Why it is still the right call for 1.0.** Signing does not switch SmartScreen
off. Reputation accrues with downloads and is separate; a freshly signed binary
from a new identity still warns. What signing buys is that reputation attaches
to the *publisher* rather than to each file, so it stops warning far sooner.
That is friction removal, and it starts mattering when strangers install rather
than friends — which is exactly the boundary decision 104 draws around 1.0.

**And self-signing was considered and rejected**, because it is the option that
looks free and is worth less than nothing: Windows still says "Unknown
publisher", SmartScreen still warns, and every user would have to install a
root certificate by hand.

**What was found while deciding**, and is worth more than the decision. There
are three cheap-or-free routes, none of them obvious: SignPath Foundation
signs qualifying open-source projects for free; Certum issues to an
*individual* for about €60–100 a year, which almost no other CA will; and the
Microsoft Store signs for you for a one-time fee and skips SmartScreen
altogether. The first answer given here was "there is no free option", which
was wrong in the way that matters — true of the obvious market and false of
the actual choices. #205 records all four so the next person starts from the
list rather than from the obvious answer.
