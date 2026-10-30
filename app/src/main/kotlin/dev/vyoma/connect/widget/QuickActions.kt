package dev.vyoma.connect.widget

import android.content.Context
import androidx.core.content.edit
import dev.vyoma.connect.R
import dev.vyoma.connect.service.Commands
import dev.vyoma.connect.ui.MainActivity

/**
 * Everything the quick-actions widget can show. Each widget instance keeps its own ordered pick
 * (long-press > Settings on One UI), so people put the buttons they use where their thumb is.
 */
enum class QuickAction(val icon: Int, val label: String, val description: String) {
    CLIPBOARD(R.drawable.ic_w_clipboard, "Clipboard", "Send what you copied to the PC"),
    FILES(R.drawable.ic_w_upload, "Send", "Pick files to send to the PC"),
    TOUCHPAD(R.drawable.ic_w_mouse, "Touchpad", "Mouse and keyboard"),
    PLAY_PAUSE(R.drawable.ic_play, "Play", "Play or pause PC media"),
    PREVIOUS(R.drawable.ic_skip_previous, "Previous", "Previous track"),
    NEXT(R.drawable.ic_skip_next, "Next", "Next track"),
    VOLUME_DOWN(R.drawable.ic_w_volume_down, "Vol −", "Turn the PC down"),
    VOLUME_UP(R.drawable.ic_w_volume_up, "Vol +", "Turn the PC up"),
    MUTE(R.drawable.ic_w_mute, "Mute PC", "Mute or unmute the PC"),
    LOCK(R.drawable.ic_w_lock, "Lock PC", "Lock the PC's screen"),
    RING(R.drawable.ic_w_ring, "Find PC", "Play a sound on the PC"),
    PING(R.drawable.ic_w_ping, "Ping", "Show a ping on the PC"),
    ;

    /** Widget button for this action; play/pause follows what's playing and carries the accent. */
    internal fun button(context: Context, s: WidgetSnapshot): Act = when (this) {
        CLIPBOARD -> Act(icon, label, Widgets.sendClipboard(context))
        FILES -> Act(icon, label, Widgets.pickFiles(context))
        TOUCHPAD -> Act(icon, label, Widgets.openApp(context, MainActivity.SCREEN_TOUCHPAD))
        PLAY_PAUSE -> Act(
            if (s.playing) R.drawable.ic_pause else R.drawable.ic_play, if (s.playing) "Pause" else "Play",
            Widgets.command(context, Commands.PLAY_PAUSE), accent = true,
        )
        PREVIOUS -> Act(icon, label, Widgets.command(context, Commands.PREVIOUS))
        NEXT -> Act(icon, label, Widgets.command(context, Commands.NEXT))
        VOLUME_DOWN -> Act(icon, label, Widgets.command(context, Commands.VOLUME_DOWN))
        VOLUME_UP -> Act(icon, label, Widgets.command(context, Commands.VOLUME_UP))
        MUTE -> Act(icon, label, Widgets.command(context, Commands.MUTE_PC))
        LOCK -> Act(icon, label, Widgets.command(context, Commands.LOCK_PC))
        RING -> Act(icon, label, Widgets.command(context, Commands.RING_PC))
        PING -> Act(icon, label, Widgets.command(context, Commands.PING))
    }

    companion object {
        /** A full 4x2 grid of the everyday ones. */
        val DEFAULT = listOf(CLIPBOARD, FILES, TOUCHPAD, PLAY_PAUSE, VOLUME_DOWN, VOLUME_UP, MUTE, LOCK)
    }
}

/** Per-widget picks, keyed by appWidgetId. Plain prefs: read synchronously while composing. */
object QuickActionStore {
    private fun prefs(context: Context) = context.getSharedPreferences("quick_actions", Context.MODE_PRIVATE)

    fun get(context: Context, appWidgetId: Int): List<QuickAction> {
        val saved = prefs(context).getString(appWidgetId.toString(), null) ?: return QuickAction.DEFAULT
        return saved.split(',').mapNotNull { name -> QuickAction.entries.firstOrNull { it.name == name } }
    }

    fun set(context: Context, appWidgetId: Int, actions: List<QuickAction>) =
        prefs(context).edit { putString(appWidgetId.toString(), actions.joinToString(",") { it.name }) }

    fun remove(context: Context, appWidgetIds: IntArray) =
        prefs(context).edit { appWidgetIds.forEach { remove(it.toString()) } }
}
