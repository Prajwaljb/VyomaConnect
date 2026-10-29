package dev.vyoma.connect.service

import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import dev.vyoma.connect.container
import kotlinx.coroutines.launch

/** Accept/Decline buttons on the pairing notification. */
class PairActionReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val deviceId = intent.getStringExtra(EXTRA_DEVICE) ?: return
        val accept = intent.getBooleanExtra(EXTRA_ACCEPT, false)
        Notifications.cancelPairRequest(context, deviceId)
        val app = context.applicationContext as android.app.Application
        val pending = goAsync()
        app.container.appScope.launch {
            try {
                val device = app.container.hub.value?.device(deviceId) ?: return@launch
                if (accept) device.acceptPairing() else device.rejectPairing()
            } finally {
                pending.finish()
            }
        }
    }

    companion object {
        private const val EXTRA_DEVICE = "device"
        private const val EXTRA_ACCEPT = "accept"

        fun intent(context: Context, deviceId: String, accept: Boolean): PendingIntent = PendingIntent.getBroadcast(
            context,
            deviceId.hashCode() * 2 + if (accept) 1 else 0,
            Intent(context, PairActionReceiver::class.java)
                .putExtra(EXTRA_DEVICE, deviceId)
                .putExtra(EXTRA_ACCEPT, accept),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
    }
}

/** Reconnect after reboot / app update. connectedDevice services may start from BOOT_COMPLETED. */
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action == Intent.ACTION_BOOT_COMPLETED || intent.action == Intent.ACTION_MY_PACKAGE_REPLACED) {
            runCatching { LinkService.start(context) }
        }
    }
}
