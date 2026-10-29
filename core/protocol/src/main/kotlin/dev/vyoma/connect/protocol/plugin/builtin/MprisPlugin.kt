package dev.vyoma.connect.protocol.plugin.builtin

import dev.vyoma.connect.protocol.NetworkPacket
import dev.vyoma.connect.protocol.PacketTypes
import dev.vyoma.connect.protocol.VLog
import dev.vyoma.connect.protocol.packet
import dev.vyoma.connect.protocol.plugin.Plugin
import dev.vyoma.connect.protocol.plugin.PluginContext
import dev.vyoma.connect.protocol.plugin.PluginSpec
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.serialization.json.put
import java.io.ByteArrayOutputStream

/** A media player on the PC, as last reported. Position is extrapolated from [positionAt] while playing. */
data class RemotePlayer(
    val name: String,
    val title: String = "",
    val artist: String = "",
    val album: String = "",
    val isPlaying: Boolean = false,
    val positionMs: Long = 0,
    val positionAt: Long = System.currentTimeMillis(),
    val lengthMs: Long = 0,
    val volume: Int = -1,
    val canPlay: Boolean = true,
    val canPause: Boolean = true,
    val canNext: Boolean = false,
    val canPrevious: Boolean = false,
    val canSeek: Boolean = false,
    val artUrl: String = "",
    /** The media URL when the player exposes it (browsers give the page URL), used for HQ art and handoff. */
    val url: String = "",
    val loopStatus: String = "",
    val shuffle: Boolean = false,
) {
    fun currentPosition(now: Long = System.currentTimeMillis()): Long {
        val p = if (isPlaying) positionMs + (now - positionAt) else positionMs
        return if (lengthMs > 0) p.coerceIn(0, lengthMs) else p.coerceAtLeast(0)
    }

    val hasMedia: Boolean get() = title.isNotBlank() || artist.isNotBlank()
}

/**
 * Controls the PC's MPRIS players. Album art arrives as a payload for local files
 * (`file://`); for web artwork the app fetches [RemotePlayer.artUrl] itself.
 */
class MprisPlugin(private val ctx: PluginContext) : Plugin {
    private val _players = MutableStateFlow<Map<String, RemotePlayer>>(emptyMap())
    val players: StateFlow<Map<String, RemotePlayer>> = _players.asStateFlow()

    /** Downloaded art keyed by the PC's art URL. Kept small: a few recent covers. */
    private val _art = MutableStateFlow<Map<String, ByteArray>>(emptyMap())
    val art: StateFlow<Map<String, ByteArray>> = _art.asStateFlow()

    private var supportsArtPayload = false
    private val requestedArt = java.util.concurrent.ConcurrentHashMap.newKeySet<String>()

    /** The player to show by default: whatever is playing, else the most recently active. */
    val activePlayer: RemotePlayer?
        get() {
            val all = _players.value.values
            return all.firstOrNull { it.isPlaying && it.hasMedia } ?: lastActive?.let { _players.value[it] } ?: all.firstOrNull { it.hasMedia }
        }
    @Volatile private var lastActive: String? = null

    override suspend fun onStart() {
        ctx.send(packet(PacketTypes.MPRIS_REQUEST) { put("requestPlayerList", true) })
    }

    override suspend fun onLinkAdded() = onStart()

    override suspend fun onPacket(packet: NetworkPacket): Boolean {
        if (packet.has("playerList")) {
            supportsArtPayload = packet.bool("supportAlbumArtPayload") ?: false
            val names = packet.stringList("playerList").toSet()
            val known = _players.value
            _players.value = names.associateWith { known[it] ?: RemotePlayer(it) }
            for (name in names - known.keys) requestState(name)
            return true
        }
        val name = packet.string("player") ?: return true

        if (packet.bool("transferringAlbumArt") == true) {
            val url = packet.string("albumArtUrl") ?: return true
            ctx.scope.launch(Dispatchers.IO) { downloadArt(url, packet) }
            return true
        }

        _players.update { map ->
            val old = map[name] ?: RemotePlayer(name)
            val now = System.currentTimeMillis()
            val updated = old.copy(
                title = packet.string("title") ?: old.title,
                artist = packet.string("artist") ?: old.artist,
                album = packet.string("album") ?: old.album,
                isPlaying = packet.bool("isPlaying") ?: old.isPlaying,
                // A new position resets the clock; otherwise carry the extrapolated one forward.
                positionMs = packet.long("pos") ?: old.currentPosition(now),
                positionAt = now,
                lengthMs = packet.long("length") ?: old.lengthMs,
                volume = packet.int("volume") ?: old.volume,
                canPlay = packet.bool("canPlay") ?: old.canPlay,
                canPause = packet.bool("canPause") ?: old.canPause,
                canNext = packet.bool("canGoNext") ?: old.canNext,
                canPrevious = packet.bool("canGoPrevious") ?: old.canPrevious,
                canSeek = packet.bool("canSeek") ?: old.canSeek,
                artUrl = packet.string("albumArtUrl") ?: old.artUrl,
                url = packet.string("url") ?: old.url,
                loopStatus = packet.string("loopStatus") ?: old.loopStatus,
                shuffle = packet.bool("shuffle") ?: old.shuffle,
            )
            if (updated.isPlaying) lastActive = name
            map + (name to updated)
        }
        _players.value[name]?.let(::maybeRequestArt)
        return true
    }

    private fun maybeRequestArt(player: RemotePlayer) {
        val url = player.artUrl
        if (url.isBlank() || !url.startsWith("file://") || !supportsArtPayload) return
        if (url in _art.value || !requestedArt.add(url)) return
        ctx.scope.launch {
            ctx.send(packet(PacketTypes.MPRIS_REQUEST) {
                put("player", player.name)
                put("albumArtUrl", url)
            })
        }
    }

    private suspend fun downloadArt(url: String, packet: NetworkPacket) {
        try {
            val bytes = ctx.device.openPayload(packet)?.use { input ->
                val out = ByteArrayOutputStream()
                val buf = ByteArray(64 * 1024)
                while (true) {
                    val n = input.read(buf)
                    if (n < 0) break
                    out.write(buf, 0, n)
                    if (out.size() > MAX_ART_BYTES) error("album art too large")
                }
                out.toByteArray()
            } ?: return
            _art.update { old -> (old + (url to bytes)).entries.toList().takeLast(MAX_ART_ENTRIES).associate { it.key to it.value } }
        } catch (e: Exception) {
            requestedArt.remove(url)
            VLog.d(TAG, "album art download failed: ${e.message}")
        }
    }

    private suspend fun requestState(player: String) = ctx.send(packet(PacketTypes.MPRIS_REQUEST) {
        put("player", player)
        put("requestNowPlaying", true)
        put("requestVolume", true)
    })

    suspend fun action(player: String, action: MediaAction) = ctx.send(packet(PacketTypes.MPRIS_REQUEST) {
        put("player", player)
        put("action", action.wire)
    }).also {
        // Flip optimistically so the UI responds instantly; the PC confirms a moment later.
        if (action == MediaAction.PLAY_PAUSE) _players.update { m ->
            val p = m[player] ?: return@update m
            m + (player to p.copy(isPlaying = !p.isPlaying, positionMs = p.currentPosition(), positionAt = System.currentTimeMillis()))
        }
    }

    suspend fun setVolume(player: String, volume: Int) = ctx.send(packet(PacketTypes.MPRIS_REQUEST) {
        put("player", player)
        put("setVolume", volume.coerceIn(0, 100))
    })

    suspend fun seekTo(player: String, positionMs: Long) = ctx.send(packet(PacketTypes.MPRIS_REQUEST) {
        put("player", player)
        put("SetPosition", positionMs.coerceAtLeast(0))
    }).also {
        _players.update { m ->
            val p = m[player] ?: return@update m
            m + (player to p.copy(positionMs = positionMs, positionAt = System.currentTimeMillis()))
        }
    }

    /** Relative seek; the wire unit is microseconds. */
    suspend fun seekBy(player: String, deltaMs: Long) = ctx.send(packet(PacketTypes.MPRIS_REQUEST) {
        put("player", player)
        put("Seek", deltaMs * 1000)
    })

    suspend fun refresh(player: String) = requestState(player)

    enum class MediaAction(val wire: String) {
        PLAY_PAUSE("PlayPause"), PLAY("Play"), PAUSE("Pause"), STOP("Stop"), NEXT("Next"), PREVIOUS("Previous"),
    }

    companion object {
        const val ID = "mpris"
        private const val TAG = "Mpris"
        private const val MAX_ART_BYTES = 8 * 1024 * 1024
        private const val MAX_ART_ENTRIES = 6

        val spec = PluginSpec(ID, incoming = setOf(PacketTypes.MPRIS), outgoing = setOf(PacketTypes.MPRIS_REQUEST), ::MprisPlugin)
    }
}
