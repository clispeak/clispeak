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
            engine.voices
                .orEmpty()
                .filter { !it.isNetworkConnectionRequired }
                .sortedBy { it.name }
                .joinToString("\n") { v ->
                    val locale = v.locale?.displayName ?: ""
                    "${v.name}\t${locale.ifEmpty { v.name }}"
                }
        } catch (_: Exception) {
            // Some engines throw rather than return an empty list.
            ""
        }
    }

    /** The voice in use, or empty for the engine's default. */
    @JvmStatic
    fun currentVoice(): String = voiceName ?: tts?.voice?.name.orEmpty()

    /** Choose a voice by name. */
    @JvmStatic
    fun setVoice(name: String): Boolean {
        val engine = tts ?: return false
        val match = engine.voices.orEmpty().firstOrNull { it.name == name } ?: return false
        val ok = engine.setVoice(match) == TextToSpeech.SUCCESS
        if (ok) voiceName = name
        return ok
    }

    /** The speaking rate. */
    @JvmStatic
    fun rate(): Float = rate

    /** Set the speaking rate. */
    @JvmStatic
    fun setRate(value: Float): Boolean {
        val engine = tts ?: return false
        val ok = engine.setSpeechRate(value) == TextToSpeech.SUCCESS
        if (ok) rate = value
        return ok
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
