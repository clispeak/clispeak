# 56. Two Android bugs, and the shell that nothing compiles until a tag

**Status:** Accepted.

Two bugs in the Kotlin, and the reason neither was caught. Issues #60 and #61.

**A pending exception killed the process.** `with_env` attaches the speech
thread to the JVM and detaches it when the guard drops. `jni` 0.21 documents
that a call returning `Err(JavaException)` leaves the exception *pending*, and
detaching a thread with one pending hands it to ART's uncaught handler, which
kills the app. Kotlin can throw here: `setVoice` and `applyPreferences` both
reach `engine.voices`, whose own comment already said some engines throw from
it. So a user picking a voice on a device with an unusual TTS engine lost the
app. Cleared on both sides now — described first, so the reason reaches
logcat rather than vanishing with the crash it was causing — and caught in
Kotlin too, because either end alone leaves the other guessing.

**`ready` was published before the listener that makes it true.** `speak`
waits on a latch the progress listener counts down, and readiness was set
before that listener was installed, so an utterance in the window registered a
latch nobody would ever release and parked the speech thread for the
five-minute timeout. Readiness is now the last thing the init callback does.

**A stop between registering a latch and queueing the text reported success.**
The stop released the latch and cleared the map, then `speak` queued the text
and returned at once — so an utterance queued *after* a stop played in full,
was reported spoken, and was then said again by the queue behind the urgent
message that interrupted it. A generation counter, bumped before the engine
call, lets `speak` see a stop that raced it. A count rather than a flag,
because two stops around one `speak` must not cancel out.

**The service stops claiming to listen when it is not.** `NodeService` holds
the foreground slot; the Activity owns the node. A `START_STICKY` restart
after Android reclaims the process therefore brings the service back with
nothing behind it, and it posted "Listening for messages" anyway — an ongoing
notification saying the device was reachable while it was not, on the platform
where being honest about that is the service's entire purpose. A null intent
is the signal, and the text becomes "Tap to start listening again", which is
also the thing to do, since tapping opens the Activity. Making the service own
the node would remove the class of problem and is a larger change; it is noted
on the issue rather than pretended away.

**Why none of this was caught, and why it is still not.** The Kotlin compiles
only on a `v*` tag, so a syntax error or a bad resource reference in the
Android shell reaches a release build with nothing having read it — the same
shape as `voicecast-app` being excluded from the Android Rust build in
decision 51.

Closing that turned out to be more expensive than it looks, and the attempt is
worth recording so the next person does not repeat it.
`./gradlew :app:compileUniversalReleaseKotlin` runs in sixteen seconds on a
machine that has already built the app, which made it look like a cheap check.
It is not, on a fresh checkout: `tauri.settings.gradle`,
`app/tauri.build.gradle.kts`, `app/tauri.properties`, `proguard-tauri.pro` and
— decisively — the whole `com/voicecast/app/generated/` package that
`MainActivity` extends are all written by the Tauri CLI and all correctly
gitignored, because they carry absolute paths. Hand-writing the first two in
CI worked and revealed the third; hand-writing the generated *sources* is not
something to attempt.

So compiling the Android shell means running the Tauri CLI's Android build,
which needs a full NDK and takes minutes rather than seconds. That is a real
cost on every pull request and a decision about CI spend rather than a tidy-up,
which is #96 rather than a step bolted onto a bug fix. The two Kotlin fixes
above were compiled locally, where the generated sources already exist.
