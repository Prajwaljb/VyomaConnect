package dev.vyoma.connect.widget

import android.annotation.SuppressLint
import android.app.PendingIntent
import android.content.Intent
import android.graphics.drawable.Icon
import android.os.Build
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService
import dev.vyoma.connect.R
import dev.vyoma.connect.container
import dev.vyoma.connect.service.Commands
import dev.vyoma.connect.ui.ClipboardActivity
import dev.vyoma.connect.ui.MainActivity
import kotlinx.coroutines.launch

/** Base: tiles show cached state on open and hand clicks to [Commands]; no network here. */
abstract class VyomaTile : TileService() {
    protected val c get() = application.container

    override fun onStartListening() = render()

    protected fun connected() = Commands.primary(c) != null

    protected fun render() {
        val tile = qsTile ?: return
        update(tile)
        tile.updateTile()
    }

    abstract fun update(tile: Tile)

    @SuppressLint("StartActivityAndCollapseDeprecated")
    protected fun open(intent: Intent) {
        intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        if (Build.VERSION.SDK_INT >= 34) {
            startActivityAndCollapse(PendingIntent.getActivity(this, intent.hashCode(), intent, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT))
        } else {
            @Suppress("DEPRECATION")
            startActivityAndCollapse(intent)
        }
    }

    protected fun command(action: String) {
        c.appScope.launch { Commands.run(this@VyomaTile, action) }
    }
}

class ClipboardTile : VyomaTile() {
    override fun update(tile: Tile) {
        tile.label = "Send clipboard"
        tile.subtitle = Commands.primary(c)?.name ?: "No PC"
        tile.icon = Icon.createWithResource(this, R.drawable.ic_w_clipboard)
        tile.state = if (connected()) Tile.STATE_INACTIVE else Tile.STATE_UNAVAILABLE
    }

    override fun onClick() = open(ClipboardActivity.intent(this))
}

class MediaTile : VyomaTile() {
    override fun update(tile: Tile) {
        val p = c.media.nowPlaying.value?.player
        tile.label = p?.title?.takeIf { it.isNotBlank() } ?: "PC media"
        tile.subtitle = p?.artist?.takeIf { it.isNotBlank() } ?: if (p == null) "Nothing playing" else null
        tile.icon = Icon.createWithResource(this, if (p?.isPlaying == true) R.drawable.ic_pause else R.drawable.ic_play)
        tile.state = when {
            p == null -> Tile.STATE_UNAVAILABLE
            p.isPlaying -> Tile.STATE_ACTIVE
            else -> Tile.STATE_INACTIVE
        }
    }

    override fun onClick() {
        command(Commands.PLAY_PAUSE)
        // Optimistic: the plugin flips isPlaying immediately.
        qsTile?.let { it.state = if (it.state == Tile.STATE_ACTIVE) Tile.STATE_INACTIVE else Tile.STATE_ACTIVE; it.updateTile() }
    }
}

class TouchpadTile : VyomaTile() {
    override fun update(tile: Tile) {
        tile.label = "Touchpad"
        tile.subtitle = Commands.primary(c)?.name ?: "No PC"
        tile.icon = Icon.createWithResource(this, R.drawable.ic_w_mouse)
        tile.state = if (connected()) Tile.STATE_INACTIVE else Tile.STATE_UNAVAILABLE
    }

    override fun onClick() = open(Intent(this, MainActivity::class.java).putExtra(MainActivity.EXTRA_SCREEN, MainActivity.SCREEN_TOUCHPAD))
}

class LockPcTile : VyomaTile() {
    override fun update(tile: Tile) {
        tile.label = "Lock PC"
        tile.subtitle = Commands.primary(c)?.name ?: "No PC"
        tile.icon = Icon.createWithResource(this, R.drawable.ic_w_lock)
        tile.state = if (connected()) Tile.STATE_INACTIVE else Tile.STATE_UNAVAILABLE
    }

    override fun onClick() {
        command(Commands.LOCK_PC)
        // It happens on the PC, out of sight: the tile says it's done until the panel closes.
        qsTile?.let { it.state = Tile.STATE_ACTIVE; it.subtitle = "Locked"; it.updateTile() }
    }
}
