# 47. Clicking the Dock icon brings the window back

**Status:** Accepted.

Closing the window hides it, because the node has to keep running for peers and
the CLI. On macOS that leaves an app with no window, and clicking its Dock icon
is the obvious way back — it did nothing. `RunEvent::Reopen` is delivered
(`applicationShouldHandleReopen`, macOS only in tauri 2.11.5) and the app called
`.run(ctx)` with no event callback at all, so every runtime event was
discarded. The tray menu was the only route back to a window someone had just
asked for, on the platform where the tray is least likely to be where they look.

`.build(ctx)?.run(|app, event| …)` now handles it by calling the same `reveal`
that `voicecast show` uses.

**Not runtime-verified, and that is worth saying.** The handler compiles against
the variant and the variant is `#[cfg(target_os = "macos")]` in the crate we
depend on, but nothing here clicked a Dock icon. Doing so needs either
accessibility permission this environment does not have, or launching a second
bundle sharing `com.voicecast.app` with the copy already installed and running.
The second is exactly the phantom above, so it was not worth risking to confirm
a five-line handler.
