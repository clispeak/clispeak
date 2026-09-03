package com.voicecast.app

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.IBinder

/**
 * Keeps the node alive while the app is in the background.
 *
 * **This service does not own the node.** The node is created and served by
 * the Activity; this only holds the foreground slot that stops Android
 * suspending the process. That split is why a `START_STICKY` restart brings
 * the service back without anything behind it — see `onStartCommand`. Making
 * the service the owner would remove the whole class of problem and is a
 * larger change than the notification text: it is noted on #60 rather than
 * pretended away here.
 *
 * Android stops an ordinary app's threads soon after it leaves the foreground,
 * which for this app means the device silently stops being reachable —
 * messages sent to it simply time out. A foreground service is the only
 * supported way to keep running, and it requires a visible notification, which
 * is a fair trade: the user can see the thing is listening and stop it.
 */
class NodeService : Service() {
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        // A null intent means Android restarted us after reclaiming the
        // process, rather than the app asking. That distinction matters: the
        // node is started by the Activity and does not exist in a freshly
        // restarted process, so this service comes back holding nothing. It
        // used to post "Listening for messages" anyway — an ongoing
        // notification saying the device was reachable while it was not, on
        // the platform where the whole point of the service is to be honest
        // about that (#60).
        val listening = intent != null
        startForeground(NOTIFICATION_ID, buildNotification(listening))
        // START_STICKY: if Android reclaims us under memory pressure, come
        // back — a device that quietly stops listening is the failure this
        // service exists to prevent.
        return START_STICKY
    }

    private fun buildNotification(listening: Boolean): Notification {
        val manager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val channel = NotificationChannel(
                CHANNEL_ID,
                "voicecast",
                // Low: this notification exists to be *available*, not to
                // interrupt. It is a status indicator, not an alert.
                NotificationManager.IMPORTANCE_LOW,
            )
            channel.description = "Shown while this device can receive spoken messages"
            manager.createNotificationChannel(channel)
        }

        val open = PendingIntent.getActivity(
            this,
            0,
            Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE,
        )

        val builder = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            Notification.Builder(this, CHANNEL_ID)
        } else {
            @Suppress("DEPRECATION")
            Notification.Builder(this)
        }

        return builder
            .setContentTitle("voicecast")
            .setContentText(
                if (listening) {
                    "Listening for messages"
                } else {
                    // Tapping opens the Activity, which is what starts the
                    // node — so this is both true and the thing to do.
                    "Tap to start listening again"
                },
            )
            .setSmallIcon(android.R.drawable.ic_lock_silent_mode_off)
            .setContentIntent(open)
            .setOngoing(true)
            .build()
    }

    companion object {
        private const val CHANNEL_ID = "voicecast.node"
        private const val NOTIFICATION_ID = 1

        /** Start the service, so this device keeps receiving in the background. */
        @JvmStatic
        fun start(context: Context) {
            val intent = Intent(context, NodeService::class.java)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                context.startForegroundService(intent)
            } else {
                context.startService(intent)
            }
        }
    }
}
