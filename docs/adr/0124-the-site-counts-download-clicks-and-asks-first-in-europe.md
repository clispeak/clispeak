# 124. The site counts download clicks, and asks first in Europe

**Status:** Accepted.

**Chosen:** clispeak.com loads Google Analytics 4 and records a
`download_click` event carrying the platform, read from the file name in the
link. Google consent mode defaults analytics to denied in the EEA, the UK and
Switzerland and to granted elsewhere; advertising storage is denied
everywhere. A banner asks visitors whose timezone is European, and a footer
button reopens the question for anyone.

**Why.** Patrick wants to know how many people download each platform. GitHub
counts asset downloads, but those numbers include every automated fetch —
this project's own release verification downloads each file, and the site's
link check fetches them too — so they cannot say how many *people* chose
macOS over Windows. A click on the site's own button can.

**Rejected: keeping the promise.** The footer said "No analytics, no cookies,
nothing loaded from anyone else", and `docs/website.md` argued for it: a
landing page that phones home about its readers undercuts software whose
claim is no server and no account. That is a real cost and it was weighed. The
decision was that the numbers are worth it, provided the page says so, which
is why the footer now does rather than going quiet.

**Rejected: server-side statistics from a DNS proxy.** No script and no
cookies, which would have kept most of the promise. It counts requests for
the page, and cannot tell which download button anyone pressed — the one
question this exists to answer.

**Rejected: loading Google's script only after a yes.** Stricter than consent
mode, since a visitor who declines would send nothing at all. But a static
page cannot know where a visitor is; it would have to decide from the
timezone alone whether to load the tag, and a European visitor with an
unexpected timezone would then be counted without being asked. Consent mode
puts the legally important half — the default — on Google's IP-based region,
and leaves only *whether to show the question* to the timezone guess. The
cost is that a visitor who declines still sends Google cookieless pings with
no identifier.

**The guess is arranged so being wrong is cheap.** An EU visitor with a
non-European timezone is never asked and, because the default was denied by
IP, is never counted. A visitor elsewhere with a European timezone is asked a
question they did not need. Neither error records anyone without a yes.

Cited from `site/index.html` and `site/consent.js`.
