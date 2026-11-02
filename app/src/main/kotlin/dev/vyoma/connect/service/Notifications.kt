package dev.vyoma.connect.service

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat
import dev.vyoma.connect.R
import dev.vyoma.connect.protocol.device.Device
import dev.vyoma.connect.protocol.device.DeviceState
import dev.vyoma.connect.platform.AndroidShareSink
import dev.vyoma.connect.protocol.plugin.builtin.Transfer
import dev.vyoma.connect.ui.ClipboardActivity
import dev.vyoma.connect.ui.FindMyPhoneActivity
import dev.vyoma.connect.ui.MainActivity
import dev.vyoma.connect.ui.ShareActivity

object Notifications {
    const val SERVICE_ID = 1
    const val MEDIA_ID = 2
    private const val TRANSFER_ID = 3
    private const val RING_ID = 4
    private const val CH_SERVICE = "service"
    private const val CH_PAIRING = "pairing"
    private const val CH_EVENTS = "events"
    const val CH_MEDIA = "media"
    private const val CH_TRANSFERS = "transfers"
    private const val CH_RECEIVED = "received"
    private const val CH_RING = "ring"

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
                NotificationChannel(CH_MEDIA, "PC media", NotificationManager.IMPORTANCE_LOW).apply {
                    description = "Control what's playing on your PC"
                    setShowBadge(false)
                },
                NotificationChannel(CH_TRANSFERS, "File transfers", NotificationManager.IMPORTANCE_LOW).apply { setShowBadge(false) },
                NotificationChannel(CH_RECEIVED, "Received files & links", NotificationManager.IMPORTANCE_HIGH),
                NotificationChannel(CH_RING, "Find my phone", NotificationManager.IMPORTANCE_HIGH).apply {
                    setSound(null, null)
                    setBypassDnd(true)
                },
            ),
        )
    }

    fun service(context: Context, connected: List<DeviceState>): Notification {
        val title = when (connected.size) {
            0 -> "Looking for your PC"
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
            .apply {
                if (connected.isNotEmpty()) {
                    addAction(0, "Send clipboard", ClipboardActivity.pending(context))
                    addAction(0, "Send files", ShareActivity.pickFilesIntent(context))
                }
            }
            .build()
    }

    // ---- transfers ------------------------------------------------------------------------------

    /**
     * One ongoing notification for everything in flight. On Android 16+ it's a promoted
     * Live Update with a progress bar, which One UI shows in the Now Bar and status chip.
     */
    fun transfers(context: Context, running: List<Transfer>) {
        val nm = context.getSystemService(NotificationManager::class.java)
        if (running.isEmpty()) {
            nm.cancel(TRANSFER_ID)
            return
        }
        val total = running.sumOf { it.totalBytes.coerceAtLeast(0) }
        val done = running.sumOf { it.doneBytes }
        val percent = if (total > 0) (done * 100 / total).toInt() else 0
        val incoming = running.all { it.incoming }
        val peer = running.first().deviceName
        val title = when {
            running.size == 1 && incoming -> "Receiving ${running.first().fileName}"
            running.size == 1 -> "Sending ${running.first().fileName}"
            incoming -> "Receiving ${running.size} files"
            else -> "Sending ${running.size} files"
        }
        val text = "${formatBytes(done)} of ${formatBytes(total)} · ${if (incoming) "from" else "to"} $peer"
        val builder = Notification.Builder(context, CH_TRANSFERS)
            .setSmallIcon(if (incoming) android.R.drawable.stat_sys_download else android.R.drawable.stat_sys_upload)
            .setContentTitle(title)
            .setContentText(text)
            .setSubText("$percent%")
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setContentIntent(openAppIntent(context))
            .setCategory(Notification.CATEGORY_PROGRESS)
        if (Build.VERSION.SDK_INT >= 36) {
            builder.setStyle(Notification.ProgressStyle().setProgress(percent).setStyledByProgress(true))
            // Live Updates came in 16 QPR2 (36.1): calling them on 16.0 throws.
            if (Build.VERSION.SDK_INT_FULL >= Build.VERSION_CODES_FULL.BAKLAVA_1) {
                builder.setShortCriticalText("$percent%")
                builder.setRequestPromotedOngoing(true)
            }
        } else {
            builder.setProgress(100, percent, total <= 0)
        }
        if (ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED) {
            nm.notify(TRANSFER_ID, builder.build())
        }
    }

    fun transferFinished(context: Context, t: Transfer) {
        val ok = t.state == Transfer.State.DONE
        val b = NotificationCompat.Builder(context, if (t.incoming && ok) CH_RECEIVED else CH_TRANSFERS)
            .setSmallIcon(R.drawable.ic_notification)
            .setAutoCancel(true)
        when {
            t.incoming && ok -> {
                val uri = t.location?.let(Uri::parse)
                b.setContentTitle("Received ${t.fileName}")
                    .setContentText("From ${t.deviceName} · ${formatBytes(t.totalBytes)} · in Downloads/Vyoma")
                if (uri != null) {
                    val view = Intent(Intent.ACTION_VIEW).setDataAndType(uri, AndroidShareSink.mimeOf(t.fileName))
                        .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
                    b.setContentIntent(PendingIntent.getActivity(context, t.id.toInt(), view, PendingIntent.FLAG_IMMUTABLE))
                    val share = Intent.createChooser(
                        Intent(Intent.ACTION_SEND).setType(AndroidShareSink.mimeOf(t.fileName)).putExtra(Intent.EXTRA_STREAM, uri)
                            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION),
                        null,
                    ).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                    b.addAction(0, "Share", PendingIntent.getActivity(context, t.id.toInt() + 7919, share, PendingIntent.FLAG_IMMUTABLE))
                }
            }
            ok -> b.setContentTitle("Sent ${t.fileName}").setContentText("To ${t.deviceName}").setTimeoutAfter(4_000)
            else -> b.setContentTitle("Couldn't ${if (t.incoming) "receive" else "send"} ${t.fileName}")
                .setContentText(if (t.incoming) "From ${t.deviceName}" else "To ${t.deviceName}. Check both devices are on the same network.")
        }
        notify(context, 5000 + (t.id % 10_000).toInt(), b.build())
    }

    // ---- things the PC sent -------------------------------------------------------------------

    fun textReceived(context: Context, from: String, text: String) = notify(
        context, ("text" + from).hashCode(),
        NotificationCompat.Builder(context, CH_RECEIVED)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle("Text from $from copied")
            .setContentText(text.take(200))
            .setStyle(NotificationCompat.BigTextStyle().bigText(text.take(1000)))
            .setAutoCancel(true)
            .setTimeoutAfter(15_000)
            .build(),
    )

    fun linkReceived(context: Context, from: String, url: String) {
        val open = Intent(Intent.ACTION_VIEW, Uri.parse(url)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        val id = ("url" + url).hashCode()
        val openPending = PendingIntent.getActivity(context, url.hashCode(), open, PendingIntent.FLAG_IMMUTABLE)
        notify(
            context, id,
            NotificationCompat.Builder(context, CH_RECEIVED)
                .setSmallIcon(R.drawable.ic_notification)
                .setContentTitle("Link from $from")
                .setContentText(url)
                .setContentIntent(openPending)
                // Often you want the link somewhere else than the browser: copying it is one tap too.
                .addAction(0, "Open", openPending)
                .addAction(0, "Copy", Commands.copyPending(context, url, id))
                .setAutoCancel(true)
                .build(),
        )
    }

    // ---- find my phone ------------------------------------------------------------------------

    fun ringing(context: Context, from: String) {
        val full = PendingIntent.getActivity(
            context, RING_ID, Intent(context, FindMyPhoneActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        notify(
            context, RING_ID,
            NotificationCompat.Builder(context, CH_RING)
                .setSmallIcon(R.drawable.ic_notification)
                .setContentTitle("$from is looking for this phone")
                .setContentText("Tap to stop ringing")
                .setCategory(Notification.CATEGORY_ALARM)
                .setPriority(NotificationCompat.PRIORITY_MAX)
                .setOngoing(true)
                .setContentIntent(full)
                .setFullScreenIntent(full, true)
                .addAction(0, "Found it", Commands.pending(context, Commands.STOP_RINGING))
                .build(),
        )
    }

    fun cancelRinging(context: Context) = context.getSystemService(NotificationManager::class.java).cancel(RING_ID)

    fun formatBytes(b: Long): String = when {
        b < 0 -> "?"
        b < 1024 -> "$b B"
        b < 1024 * 1024 -> "${b / 1024} KB"
        b < 1024L * 1024 * 1024 -> "%.1f MB".format(b / 1048576.0)
        else -> "%.2f GB".format(b / 1073741824.0)
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

    fun openAppIntent(context: Context, deviceId: String? = null): PendingIntent = openApp(context, deviceId)

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
