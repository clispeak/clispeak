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
