package com.voicecast.app

/**
 * An invite opened from outside the app, waiting to be collected.
 *
 * A QR scan launches the activity with the invite in its intent, which can
 * happen before the interface exists. Parking it here lets the UI pick it up
 * whenever it is ready, and clears it so a rotation does not rejoin.
 */
object Invites {
    @Volatile
    @JvmStatic
    var pending: String? = null

    /** Take the pending invite, if any, clearing it. */
    @JvmStatic
    fun take(): String? {
        val value = pending
        pending = null
        return value
    }
}
