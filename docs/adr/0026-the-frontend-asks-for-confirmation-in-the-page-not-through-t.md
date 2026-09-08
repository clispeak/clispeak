# 26. The frontend asks for confirmation in the page, not through the webview

**Status:** Accepted.

**Decision.** Destructive actions confirm through an in-page dialog. A gate in
`cargo xtask portability` fails the build if `confirm`, `alert` or `prompt`
appears in the frontend.

**Why.** `window.confirm` is not portable. WebKitGTK and WebView2 show it;
WKWebView shows a script dialog only if the host implements a `WKUIDelegate`
for it, and wry's delegate implements file upload, media capture permission
and new windows — no confirm panel. So `confirm()` returned false with nothing
displayed, and every action behind one returned at its first line while its
button reported success.

Five actions: clearing the history, removing a device, and dropping, leaving
or rotating a space. **Rotate is why this is more than a defect.** It is the
answer to a stolen device, and someone could believe they had locked one out
of a space while nothing whatsoever had happened.

**How far it reaches.** macOS and iOS, which share that backend — iOS has
simply never been run. Android is unaffected: wry's `RustWebChromeClient`
overrides `onJsConfirm` and builds a real dialog. Linux and Windows were
always fine, which is exactly why this survived so long: it was invisible on
the two platforms getting daily use.

**Why not `@tauri-apps/plugin-dialog`.** It would add a dependency, a
capability entry and a bundler step, to arrive at a dialog whose behaviour
still varies by platform. The in-page one is identical on all five.

**Why a mechanical gate rather than a note.** The symptom is a button that
looks like it worked. Nothing fails, nothing logs, and the only way to notice
is to check afterwards whether the thing happened — which is precisely what
nobody does with a confirmation dialog. The rule is the same one
`voicecast-core` already has one layer down: a platform assumption in shared
code, caught by a check rather than by a reader.
