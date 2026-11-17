package dev.vyoma.connect.widget

import android.content.Context
import androidx.core.content.edit
import androidx.glance.action.Action
import dev.vyoma.connect.R
import dev.vyoma.connect.service.Commands
import dev.vyoma.connect.ui.MainActivity

/**
 * Everything the Shortcuts widget can show. Each widget instance keeps its own ordered pick
 * (long-press > Settings on One UI), so people put the buttons they use where their thumb is.
 */
enum class QuickAction(val icon: Int, val label: String, val description: String, internal val hue: Hue) {
    FILES(R.drawable.ic_w_upload, "Send files", "Pick files to send to the PC", Hue.BLUE),
    CLIPBOARD(R.drawable.ic_w_clipboard, "Send clip", "Send what you copied to the PC", Hue.TEAL),
    TOUCHPAD(R.drawable.ic_w_mouse, "Touchpad", "Mouse and keyboard", Hue.ORANGE),
    PLAY_PAUSE(R.drawable.ic_play, "Play", "Play or pause PC media", Hue.PINK),
    PREVIOUS(R.drawable.ic_skip_previous, "Previous", "Previous track", Hue.PINK),
    NEXT(R.drawable.ic_skip_next, "Next", "Next track", Hue.PINK),
    VOLUME_DOWN(R.drawable.ic_w_volume_down, "Quieter", "Turn the PC down", Hue.GRAY),
    VOLUME_UP(R.drawable.ic_w_volume_up, "Louder", "Turn the PC up", Hue.GRAY),
    MUTE(R.drawable.ic_w_mute, "Mute", "Mute or unmute the PC", Hue.GRAY),
    LOCK(R.drawable.ic_w_lock, "Lock PC", "Lock the PC's screen", Hue.MINT),
    PING(R.drawable.ic_w_ping, "Ping", "Show a ping on the PC", Hue.CYAN),
    ;

    /** What a tap on this tile does. */
    internal fun action(context: Context): Action = when (this) {
        FILES -> Widgets.pickFiles(context)
        CLIPBOARD -> Widgets.sendClipboard(context)
        TOUCHPAD -> Widgets.openApp(context, MainActivity.SCREEN_TOUCHPAD)
        PLAY_PAUSE -> Widgets.command(context, Commands.PLAY_PAUSE)
        PREVIOUS -> Widgets.command(context, Commands.PREVIOUS)
        NEXT -> Widgets.command(context, Commands.NEXT)
        VOLUME_DOWN -> Widgets.command(context, Commands.VOLUME_DOWN)
        VOLUME_UP -> Widgets.command(context, Commands.VOLUME_UP)
        MUTE -> Widgets.command(context, Commands.MUTE_PC)
        LOCK -> Widgets.command(context, Commands.LOCK_PC)
        PING -> Widgets.command(context, Commands.PING)
    }

    companion object {
        /** The everyday ones, Send files first: on a 1x1 the first pick is the whole widget. */
        val DEFAULT = listOf(FILES, CLIPBOARD, TOUCHPAD, PLAY_PAUSE, LOCK, MUTE, VOLUME_DOWN, VOLUME_UP)
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
