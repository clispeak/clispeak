package com.voicecast.app

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import androidx.activity.enableEdgeToEdge
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat

class MainActivity : TauriActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)

        // Started here rather than from Rust so the engine is created with an
        // Activity context and Rust never has to hold a jobject.
        Speech.init(this)
        Battery.attach(this)

        requestNotificationPermission()
        NodeService.start(this)

        capturePendingInvite(intent)
    }

    /**
     * The activity is singleTask, so a scan while it is already open arrives
     * here rather than through onCreate.
     */
    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        capturePendingInvite(intent)
    }

    /**
     * Hold an invite opened from a QR scan until the interface asks for it.
     *
     * The webview may not be ready when the intent arrives, so this parks the
     * value rather than trying to hand it over immediately.
     */
    private fun capturePendingInvite(intent: Intent?) {
        val data = intent?.data ?: return
        if (data.scheme == "voicecast") {
            Invites.pending = data.toString()
        }
    }

    /**
     * A foreground service needs a visible notification, and from Android 13
     * showing one needs permission. Without it the service still runs, but
     * silently — so ask, and carry on either way.
     */
    private fun requestNotificationPermission() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return
        val granted = ContextCompat.checkSelfPermission(
            this,
            Manifest.permission.POST_NOTIFICATIONS,
        ) == PackageManager.PERMISSION_GRANTED
        if (!granted) {
            ActivityCompat.requestPermissions(
                this,
                arrayOf(Manifest.permission.POST_NOTIFICATIONS),
                1,
            )
        }
    }
}
