package dev.vyoma.connect.platform

import android.content.ComponentName
import android.content.Context
import android.media.session.MediaController
import android.media.session.MediaSessionManager
import android.media.session.PlaybackState
import android.net.Uri
import android.os.SystemClock

/**
 * Links that open at a moment in a video. YouTube (and its short links, Shorts and music
 * domain) take `t=<seconds>`; the YouTube app honours it when opened from a link.
 */
object Handoff {
    private val youtubeHosts = setOf("youtube.com", "www.youtube.com", "m.youtube.com", "music.youtube.com", "youtu.be")

    fun isYouTube(url: String): Boolean = runCatching { Uri.parse(url).host?.lowercase() in youtubeHosts }.getOrDefault(false)

    /** [url] set to start at [positionMs] if it's a YouTube video; other links are returned as they are. */
    fun at(url: String, positionMs: Long): String {
        if (!isYouTube(url)) return url
        val uri = Uri.parse(url)
        val id = when {
            uri.host.equals("youtu.be", ignoreCase = true) -> uri.pathSegments.firstOrNull()
            uri.pathSegments.firstOrNull() in setOf("shorts", "live", "embed") -> uri.pathSegments.getOrNull(1)
            else -> uri.getQueryParameter("v")
        } ?: return url
        val seconds = (positionMs / 1000).coerceAtLeast(0)
        val base = if (uri.host.equals("music.youtube.com", ignoreCase = true)) "https://music.youtube.com/watch" else "https://www.youtube.com/watch"
        val list = uri.getQueryParameter("list")?.let { "&list=$it" }.orEmpty()
        return "$base?v=$id$list&t=${seconds}s"
    }

    private val youtubeApps = setOf("com.google.android.youtube", "com.google.android.apps.youtube.music", "app.revanced.android.youtube")

    /**
     * Sharing a YouTube link to the PC while that video plays here: the link, set to the moment
     * the phone is at, and the phone pauses so the PC takes over. Anything else passes through.
     * Needs notification access (Vyoma has it for mirroring); without it the link goes as is.
     */
    fun continueOnPc(context: Context, url: String): String {
        if (!isYouTube(url)) return url
        val controller = youtubeSession(context) ?: return url
        val state = controller.playbackState ?: return url
        val playing = state.state == PlaybackState.STATE_PLAYING
        if (!playing && state.state != PlaybackState.STATE_PAUSED) return url
        val since = if (playing) (SystemClock.elapsedRealtime() - state.lastPositionUpdateTime) * state.playbackSpeed else 0f
        val position = state.position + since.toLong()
        if (playing) controller.transportControls.pause()
        return at(url, position)
    }

    private fun youtubeSession(context: Context): MediaController? = runCatching {
        val sessions = context.getSystemService(MediaSessionManager::class.java)
            .getActiveSessions(ComponentName(context, VyomaNotificationListener::class.java))
        sessions.firstOrNull { it.packageName in youtubeApps && it.playbackState?.state == PlaybackState.STATE_PLAYING }
            ?: sessions.firstOrNull { it.packageName in youtubeApps }
    }.getOrNull() // no notification access
}
