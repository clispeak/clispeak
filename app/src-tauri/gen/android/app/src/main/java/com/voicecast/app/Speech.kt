package com.voicecast.app

import android.content.Context
import android.speech.tts.TextToSpeech
import android.util.Log
import java.util.Locale

/**
 * Wraps Android's TextToSpeech for the Rust core to call.
 *
 * The wrapping exists because TextToSpeech initialises asynchronously through
 * a listener callback, which is painful to drive from raw JNI. Keeping that
 * dance in Kotlin leaves Rust with four plain static calls.
 *
 * The engine is created here with the Activity's context rather than being
 * passed one from Rust, so Rust never has to hold a jobject.
 */
object Speech {
    private const val TAG = "voicecast"

    private var tts: TextToSpeech? = null

    /** Set once the engine reports it initialised successfully. */
    @Volatile
    private var ready = false

    /** Why speech is unavailable, in words a person could act on. */
    @Volatile
    private var failure: String? = "the speech engine is still starting"

    /** Create the engine. Safe to call more than once. */
    @JvmStatic
    fun init(context: Context) {
        if (tts != null) return
        tts = TextToSpeech(context.applicationContext) { status ->
            if (status == TextToSpeech.SUCCESS) {
                val result = tts?.setLanguage(Locale.getDefault())
                if (result == TextToSpeech.LANG_MISSING_DATA ||
                    result == TextToSpeech.LANG_NOT_SUPPORTED
                ) {
                    // Fall back rather than refuse: a voice in the wrong
                    // locale is far better than silence.
                    tts?.setLanguage(Locale.US)
                }
                ready = true
                failure = null
                // Apply anything chosen before the engine existed.
                // TextToSpeech initialises asynchronously, so a preference
                // restored at startup arrives before there is anything to
                // apply it to — and was silently dropped.
                applyPreferences()
                Log.i(TAG, "speech engine ready")
            } else {
                failure = "this device has no working text-to-speech engine"
                Log.w(TAG, "speech engine failed to initialise: $status")
            }
        }
    }

    /** Speaking rate, where 1.0 is the engine's normal pace. */
    @Volatile
    private var rate = 1.0f

    /** The chosen voice's name, or null for the engine's default. */
    @Volatile
    private var voiceName: String? = null

    /**
     * Installed voices, newline-separated as `name\tlabel`.
     *
     * Flattened to a string because returning a list across JNI costs far
     * more code than splitting one here.
     */
    @JvmStatic
    fun voices(): String {
        val engine = tts ?: return ""
        return try {
            val language = Locale.getDefault().language
            val usable = engine.voices.orEmpty().filter { !it.isNetworkConnectionRequired }

            // This device offers 133 voices across dozens of languages. A
            // dropdown of all of them is unusable, and a voice in the wrong
            // language reading English is worse than having no choice — so
            // only the device's own language is offered. If somehow none
            // match, fall back to everything rather than an empty picker.
            val matching = usable.filter { it.locale?.language == language }
            val offered = matching.ifEmpty { usable }

            offered
                .sortedWith(
                    // The exact locale first, then the rest of the language.
                    compareBy(
                        { if (it.locale?.toString() == Locale.getDefault().toString()) 0 else 1 },
                        { it.locale?.toString() ?: "" },
                        { it.name },
                    ),
                )
                .joinToString("\n") { v -> "${v.name}\t${label(v)}" }
        } catch (_: Exception) {
            // Some engines throw rather than return an empty list.
            ""
        }
    }

    /**
     * A label that actually distinguishes one voice from another.
     *
     * Android names voices like `en-us-x-iom-local`, which is unreadable, and
     * the locale alone is useless — most devices ship several voices per
     * language, so labelling by locale makes the list look like a language
     * picker that does nothing. The variant token is the only thing that
     * differs, so it has to be shown.
     */
    private fun label(voice: android.speech.tts.Voice): String {
        // en-us-x-iom-local -> iom
        val variant = voice.name
            .split("-")
            .dropWhile { it != "x" }
            .drop(1)
            .firstOrNull()
            ?.uppercase()
        // The list is already filtered to one language, so repeating the
        // locale on every row costs width and wraps each entry onto two
        // lines. Show it only where it actually differs from this device's.
        val here = Locale.getDefault().toString()
        val elsewhere = voice.locale?.toString() != here
        val place = voice.locale?.getDisplayCountry(Locale.getDefault()).orEmpty()

        return when {
            variant.isNullOrBlank() && elsewhere -> "Default ($place)"
            variant.isNullOrBlank() -> "Default voice"
            elsewhere && place.isNotEmpty() -> "Voice $variant ($place)"
            else -> "Voice $variant"
        }
    }

    /** The voice in use, or empty for the engine's default. */
    @JvmStatic
    fun currentVoice(): String = voiceName ?: tts?.voice?.name.orEmpty()

    /**
     * Choose a voice by name.
     *
     * Remembered even when the engine is not ready yet, and applied once it
     * is. Refusing here would lose a preference restored at startup, which is
     * exactly when the engine is still initialising.
     */
    @JvmStatic
    fun setVoice(name: String): Boolean {
        voiceName = name
        val engine = tts ?: return true
        val match = engine.voices.orEmpty().firstOrNull { it.name == name } ?: return false
        return engine.setVoice(match) == TextToSpeech.SUCCESS
    }

    /** The speaking rate. */
    @JvmStatic
    fun rate(): Float = rate

    /** Set the speaking rate. Remembered even before the engine is ready. */
    @JvmStatic
    fun setRate(value: Float): Boolean {
        rate = value
        val engine = tts ?: return true
        return engine.setSpeechRate(value) == TextToSpeech.SUCCESS
    }

    /** Apply whatever was chosen, now that there is an engine to apply it to. */
    private fun applyPreferences() {
        val engine = tts ?: return
        engine.setSpeechRate(rate)
        voiceName?.let { name ->
            engine.voices.orEmpty().firstOrNull { it.name == name }?.let { engine.setVoice(it) }
        }
    }

    /** Whether speech can be attempted right now. */
    @JvmStatic
    fun isReady(): Boolean = ready

    /** Why speech is unavailable, or null when it is. */
    @JvmStatic
    fun failureReason(): String? = failure

    /**
     * Speak one chunk, queued behind anything already speaking.
     *
     * QUEUE_ADD rather than QUEUE_FLUSH: the core has already split the
     * message into chunks and expects them spoken in order.
     */
    @JvmStatic
    fun speak(text: String): Boolean {
        val engine = tts ?: return false
        if (!ready) return false
        val result = engine.speak(text, TextToSpeech.QUEUE_ADD, null, "voicecast")
        return result == TextToSpeech.SUCCESS
    }

    /** Stop immediately, discarding anything queued. */
    @JvmStatic
    fun stop() {
        tts?.stop()
    }

    /** Whether the engine is still speaking. */
    @JvmStatic
    fun isSpeaking(): Boolean = tts?.isSpeaking ?: false

    /** Release the engine. */
    @JvmStatic
    fun shutdown() {
        tts?.shutdown()
        tts = null
        ready = false
        failure = "the speech engine was shut down"
    }
}
