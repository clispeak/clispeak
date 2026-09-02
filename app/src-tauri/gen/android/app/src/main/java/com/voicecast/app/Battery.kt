package com.voicecast.app

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.PowerManager
import android.provider.Settings

/**
 * Battery optimisation, which decides whether this device stays reachable.
 *
 * A foreground service keeps the process alive, but Doze can still suspend its
 * network once the screen has been off a while — so a message sent to a
 * sleeping phone simply never arrives. Exempting the app is the only reliable
 * fix, and it needs the user's explicit consent.
 */
object Battery {
    /** The activity to launch the system dialog from. */
    private var activity: Activity? = null

    /** Remember the activity so a request can be raised later, from Rust. */
    @JvmStatic
    fun attach(activity: Activity) {
        this.activity = activity
    }

    /** Whether this app is already exempt from battery optimisation. */
    @JvmStatic
    fun isExempt(): Boolean {
        val ctx = activity ?: return false
        val power = ctx.getSystemService(Context.POWER_SERVICE) as PowerManager
        return power.isIgnoringBatteryOptimizations(ctx.packageName)
    }

    /**
     * Ask the user to exempt this app.
     *
     * Prefers the direct dialog, which is one tap. Falls back to the settings
     * list if that intent is unavailable — better a couple of taps than a
     * dead end.
     */
    @JvmStatic
    fun requestExemption(): Boolean {
        val ctx = activity ?: return false
        return try {
            val intent = Intent(
                Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS,
                Uri.parse("package:${ctx.packageName}"),
            )
            ctx.startActivity(intent)
            true
        } catch (_: Exception) {
            try {
                ctx.startActivity(Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS))
                true
            } catch (_: Exception) {
                false
            }
        }
    }
}
