package dev.vyoma.connect.service

import android.app.Application
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import dev.vyoma.connect.AppContainer
import dev.vyoma.connect.container
import dev.vyoma.connect.platform.Ringer
import dev.vyoma.connect.protocol.device.Device
import dev.vyoma.connect.protocol.plugin.builtin.FindMyPhonePlugin
import dev.vyoma.connect.protocol.plugin.builtin.LockPlugin
import dev.vyoma.connect.protocol.plugin.builtin.MprisPlugin
import dev.vyoma.connect.protocol.plugin.builtin.PingPlugin
import dev.vyoma.connect.protocol.plugin.builtin.Transfers
import kotlinx.coroutines.launch

/**
 * One-shot actions shared by notifications, widgets, tiles and shortcuts. They target the
 * given device, or the "primary" one: the first paired device that's connected.
 */
object Commands {
    const val PLAY_PAUSE = "dev.vyoma.connect.cmd.PLAY_PAUSE"
    const val NEXT = "dev.vyoma.connect.cmd.NEXT"
    const val PREVIOUS = "dev.vyoma.connect.cmd.PREVIOUS"
    const val RING_PC = "dev.vyoma.connect.cmd.RING_PC"
    const val LOCK_PC = "dev.vyoma.connect.cmd.LOCK_PC"
    const val PING = "dev.vyoma.connect.cmd.PING"
    const val STOP_RINGING = "dev.vyoma.connect.cmd.STOP_RINGING"
    const val DISMISS_TRANSFERS = "dev.vyoma.connect.cmd.DISMISS_TRANSFERS"
    const val EXTRA_DEVICE = "device"

    fun pending(context: Context, action: String, deviceId: String? = null): PendingIntent = PendingIntent.getBroadcast(
        context,
        (action + deviceId).hashCode(),
        Intent(context, CommandReceiver::class.java).setAction(action).apply { deviceId?.let { putExtra(EXTRA_DEVICE, it) } },
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    fun primary(container: AppContainer): Device? = container.hub.value?.devices?.value?.values
        ?.filter { it.state.value.isPaired && it.state.value.isReachable }
        ?.sortedBy { it.state.value.transports.minOf { t -> t.ordinal } }
        ?.firstOrNull()

    suspend fun run(context: Context, action: String, deviceId: String? = null) {
        val app = context.applicationContext as Application
        val c = app.container
        when (action) {
            STOP_RINGING -> { Ringer.stop(app); Notifications.cancelRinging(app); return }
            DISMISS_TRANSFERS -> { Transfers.dismissFinished(); return }
            PLAY_PAUSE -> { c.media.control { p, player -> p.action(player, MprisPlugin.MediaAction.PLAY_PAUSE) }; return }
            NEXT -> { c.media.control { p, player -> p.action(player, MprisPlugin.MediaAction.NEXT) }; return }
            PREVIOUS -> { c.media.control { p, player -> p.action(player, MprisPlugin.MediaAction.PREVIOUS) }; return }
        }
        val device = deviceId?.let { c.hub.value?.device(it) } ?: primary(c) ?: return
        when (action) {
            RING_PC -> device.plugin<FindMyPhonePlugin>(FindMyPhonePlugin.ID)?.ringPeer()
            LOCK_PC -> device.plugin<LockPlugin>(LockPlugin.ID)?.setLocked(true)
            PING -> device.plugin<PingPlugin>(PingPlugin.ID)?.ping("Ping from Vyoma")
        }
    }
}

class CommandReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val action = intent.action ?: return
        val pending = goAsync()
        val app = context.applicationContext as Application
        app.container.appScope.launch {
            try {
                Commands.run(app, action, intent.getStringExtra(Commands.EXTRA_DEVICE))
            } finally {
                pending.finish()
            }
        }
    }
}
