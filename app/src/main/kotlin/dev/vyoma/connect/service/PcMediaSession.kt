package dev.vyoma.connect.service

import android.app.Notification
import android.app.NotificationManager
import android.content.Context
import android.graphics.drawable.Icon
import android.media.MediaMetadata
import android.media.session.MediaSession
import android.media.session.PlaybackState
import android.media.VolumeProvider
import dev.vyoma.connect.R
import dev.vyoma.connect.platform.MediaHub
import dev.vyoma.connect.platform.NowPlaying
import dev.vyoma.connect.protocol.plugin.builtin.MprisPlugin
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.launch

/**
 * Mirrors the PC's player as a real Android media session, so it shows up where One UI puts
 * media: the notification panel player, lock screen, Now Bar, and Bluetooth/watch controls.
 * Phone volume keys adjust the PC player's volume while it's the active session.
 */
class PcMediaSession(private val context: Context, private val media: MediaHub) {
    private val session = MediaSession(context, "vyoma-pc").apply {
        setCallback(object : MediaSession.Callback() {
            override fun onPlay() = media.control { p, n -> p.action(n, MprisPlugin.MediaAction.PLAY) }
            override fun onPause() = media.control { p, n -> p.action(n, MprisPlugin.MediaAction.PAUSE) }
            override fun onSkipToNext() = media.control { p, n -> p.action(n, MprisPlugin.MediaAction.NEXT) }
            override fun onSkipToPrevious() = media.control { p, n -> p.action(n, MprisPlugin.MediaAction.PREVIOUS) }
            override fun onStop() = media.control { p, n -> p.action(n, MprisPlugin.MediaAction.STOP) }
            override fun onSeekTo(pos: Long) = media.control { p, n -> p.seekTo(n, pos) }
        })
    }
    private var job: Job? = null
    private var lastArt: Any? = null
    private var volume: PcVolume? = null

    fun start(scope: CoroutineScope) {
        job = scope.launch {
            media.nowPlaying
                .distinctUntilChanged { a, b -> a?.player == b?.player && a?.art === b?.art && a?.deviceId == b?.deviceId }
                .collect(::render)
        }
    }

    fun stop() {
        job?.cancel()
        session.isActive = false
        session.release()
        cancel()
    }

    private fun render(np: NowPlaying?) {
        if (np == null || !np.player.hasMedia) {
            session.isActive = false
            cancel()
            return
        }
        val p = np.player
        if (lastArt !== np.art || session.controller.metadata?.getString(MediaMetadata.METADATA_KEY_TITLE) != p.title) {
            session.setMetadata(
                MediaMetadata.Builder()
                    .putString(MediaMetadata.METADATA_KEY_TITLE, p.title)
                    .putString(MediaMetadata.METADATA_KEY_ARTIST, p.artist)
                    .putString(MediaMetadata.METADATA_KEY_ALBUM, p.album)
                    .putLong(MediaMetadata.METADATA_KEY_DURATION, if (p.lengthMs > 0) p.lengthMs else -1)
                    .apply { np.art?.let { putBitmap(MediaMetadata.METADATA_KEY_ART, it) } }
                    .build(),
            )
            lastArt = np.art
        }
        var actions = PlaybackState.ACTION_PLAY_PAUSE or PlaybackState.ACTION_PLAY or PlaybackState.ACTION_PAUSE or PlaybackState.ACTION_STOP
        if (p.canNext) actions = actions or PlaybackState.ACTION_SKIP_TO_NEXT
        if (p.canPrevious) actions = actions or PlaybackState.ACTION_SKIP_TO_PREVIOUS
        if (p.canSeek && p.lengthMs > 0) actions = actions or PlaybackState.ACTION_SEEK_TO
        session.setPlaybackState(
            PlaybackState.Builder()
                .setActions(actions)
                .setState(if (p.isPlaying) PlaybackState.STATE_PLAYING else PlaybackState.STATE_PAUSED, p.positionMs, if (p.isPlaying) 1f else 0f, p.positionAt.toElapsed())
                .build(),
        )
        if (p.volume >= 0) {
            val v = volume ?: PcVolume().also { volume = it; session.setPlaybackToRemote(it) }
            v.currentVolume = p.volume
        }
        session.isActive = true
        notify(np)
    }

    /** SystemClock-based timestamp the framework uses to extrapolate the seek bar. */
    private fun Long.toElapsed(): Long = android.os.SystemClock.elapsedRealtime() - (System.currentTimeMillis() - this)

    private inner class PcVolume : VolumeProvider(VOLUME_CONTROL_ABSOLUTE, 100, 50) {
        override fun onSetVolumeTo(volume: Int) {
            currentVolume = volume
            media.control { p, n -> p.setVolume(n, volume) }
        }

        override fun onAdjustVolume(direction: Int) = onSetVolumeTo((currentVolume + direction * 5).coerceIn(0, 100))
    }

    private fun notify(np: NowPlaying) {
        val p = np.player
        val nm = context.getSystemService(NotificationManager::class.java)
        if (!nm.areNotificationsEnabled()) return
        fun action(icon: Int, title: String, cmd: String) =
            Notification.Action.Builder(Icon.createWithResource(context, icon), title, Commands.pending(context, cmd)).build()
        val n = Notification.Builder(context, Notifications.CH_MEDIA)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(p.title.ifBlank { p.name })
            .setContentText(listOf(p.artist, np.deviceName).filter { it.isNotBlank() }.joinToString(" · "))
            .setSubText(np.deviceName)
            .setLargeIcon(np.art)
            .setContentIntent(Notifications.openAppIntent(context, np.deviceId))
            .setOngoing(p.isPlaying)
            .setShowWhen(false)
            .setVisibility(Notification.VISIBILITY_PUBLIC)
            .addAction(action(R.drawable.ic_skip_previous, "Previous", Commands.PREVIOUS))
            .addAction(
                if (p.isPlaying) action(R.drawable.ic_pause, "Pause", Commands.PLAY_PAUSE)
                else action(R.drawable.ic_play, "Play", Commands.PLAY_PAUSE),
            )
            .addAction(action(R.drawable.ic_skip_next, "Next", Commands.NEXT))
            .setStyle(Notification.MediaStyle().setMediaSession(session.sessionToken).setShowActionsInCompactView(0, 1, 2))
            .build()
        nm.notify(Notifications.MEDIA_ID, n)
    }

    private fun cancel() = context.getSystemService(NotificationManager::class.java).cancel(Notifications.MEDIA_ID)
}
