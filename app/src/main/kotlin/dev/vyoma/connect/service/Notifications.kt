package dev.vyoma.connect.service

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat
import dev.vyoma.connect.R
import dev.vyoma.connect.protocol.device.Device
import dev.vyoma.connect.protocol.device.DeviceState
import dev.vyoma.connect.ui.MainActivity

object Notifications {
    const val SERVICE_ID = 1
    private const val CH_SERVICE = "service"
    private const val CH_PAIRING = "pairing"
    private const val CH_EVENTS = "events"

    fun ensureChannels(context: Context) {
        val nm = context.getSystemService(NotificationManager::class.java)
        nm.createNotificationChannels(
            listOf(
                NotificationChannel(CH_SERVICE, "Connection status", NotificationManager.IMPORTANCE_MIN).apply {
                    description = "Quiet ongoing notification that keeps devices connected"
                    setShowBadge(false)
                },
                NotificationChannel(CH_PAIRING, "Pairing requests", NotificationManager.IMPORTANCE_HIGH),
                NotificationChannel(CH_EVENTS, "Pings & alerts", NotificationManager.IMPORTANCE_DEFAULT),
            ),
        )
    }

    fun service(context: Context, connected: List<DeviceState>): Notification {
        val title = when (connected.size) {
            0 -> "Looking for devices"
            1 -> "Connected to ${connected.first().identity.deviceName}"
            else -> "Connected to ${connected.size} devices"
        }
        return NotificationCompat.Builder(context, CH_SERVICE)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(title)
            .setContentIntent(openApp(context))
            .setOngoing(true)
            .setSilent(true)
            .setShowWhen(false)
            .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
            .setCategory(Notification.CATEGORY_SERVICE)
            .build()
    }

    fun updateService(context: Context, connected: List<DeviceState>) =
        notify(context, SERVICE_ID, service(context, connected))

    fun pairRequest(context: Context, device: Device) {
        val key = device.state.value.verificationKey
        val n = NotificationCompat.Builder(context, CH_PAIRING)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle("Pair with ${device.name}?")
            .setContentText(if (key != null) "Check the code matches: $key" else "Open Vyoma to respond")
            .setContentIntent(openApp(context, device.id))
            .setAutoCancel(true)
            .setTimeoutAfter(30_000)
            .setCategory(Notification.CATEGORY_RECOMMENDATION)
            .addAction(0, "Decline", PairActionReceiver.intent(context, device.id, accept = false))
            .addAction(0, "Pair", PairActionReceiver.intent(context, device.id, accept = true))
            .build()
        notify(context, pairId(device.id), n)
    }

    fun cancelPairRequest(context: Context, deviceId: String) =
        context.getSystemService(NotificationManager::class.java).cancel(pairId(deviceId))

    fun ping(context: Context, deviceName: String, message: String?) = notify(
        context, ("ping" + deviceName).hashCode(),
        NotificationCompat.Builder(context, CH_EVENTS)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle("Ping from $deviceName")
            .setContentText(message ?: "👋")
            .setAutoCancel(true)
            .build(),
    )

    fun certificateMismatch(context: Context, deviceName: String) = notify(
        context, ("cert" + deviceName).hashCode(),
        NotificationCompat.Builder(context, CH_EVENTS)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle("$deviceName's identity changed")
            .setContentText("Its security key no longer matches. Unpair and pair again if you reinstalled KDE Connect.")
            .setStyle(NotificationCompat.BigTextStyle())
            .setContentIntent(openApp(context))
            .setAutoCancel(true)
            .build(),
    )

    private fun pairId(deviceId: String) = 1000 + (deviceId.hashCode() and 0xFFFF)

    private fun openApp(context: Context, deviceId: String? = null): PendingIntent {
        val intent = Intent(context, MainActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP)
            .apply { if (deviceId != null) putExtra(MainActivity.EXTRA_DEVICE, deviceId) }
        return PendingIntent.getActivity(
            context, deviceId?.hashCode() ?: 0, intent,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
    }

    private fun notify(context: Context, id: Int, notification: Notification) {
        if (ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) return
        context.getSystemService(NotificationManager::class.java).notify(id, notification)
    }
}
