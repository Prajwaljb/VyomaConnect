package dev.vyoma.connect.platform

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.util.LruCache
import dev.vyoma.connect.protocol.device.Device
import dev.vyoma.connect.protocol.device.DeviceHub
import dev.vyoma.connect.protocol.plugin.builtin.MprisPlugin
import dev.vyoma.connect.protocol.plugin.builtin.RemotePlayer
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.flow.flatMapLatest
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.flow.transformLatest
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.net.HttpURLConnection
import java.net.URI
import java.net.URL

/** What the PC is playing right now, ready for the UI, media session, widget and tile. */
data class NowPlaying(
    val deviceId: String,
    val deviceName: String,
    val player: RemotePlayer,
    val art: Bitmap?,
    val players: List<String>,
)

/**
 * Picks the one PC player worth showing (whatever is playing, else the user's pick, else the
 * last active) and resolves the best artwork we can get for it.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class MediaHub(private val hub: StateFlow<DeviceHub?>, private val scope: CoroutineScope) {
    /** The player the user chose in the UI; playing players still win. */
    val preferredPlayer = MutableStateFlow<String?>(null)

    private class Source(val device: Device, val plugin: MprisPlugin, val player: RemotePlayer, val embeddedArt: ByteArray?, val players: List<String>)

    private val source: Flow<Source?> = hub.filterNotNull()
        .flatMapLatest { it.devices }
        .flatMapLatest { map ->
            if (map.isEmpty()) flowOf(emptyList())
            else combine(map.values.map { d -> d.state.map { s -> d to (s.isReachable && MprisPlugin.ID in s.plugins) } }) { it.toList() }
        }
        .map { pairs -> pairs.filter { it.second }.mapNotNull { (d, _) -> d.plugin<MprisPlugin>(MprisPlugin.ID)?.let { d to it } } }
        .distinctUntilChanged { a, b -> a.map { it.second } == b.map { it.second } }
        .flatMapLatest { plugins ->
            if (plugins.isEmpty()) flowOf(null)
            else {
                val perDevice = combine(plugins.map { (d, p) -> combine(p.players, p.art) { players, art -> Triple(d, p, players to art) } }) { it.toList() }
                combine(perDevice, preferredPlayer) { all, preferred -> pick(all, preferred) }
            }
        }

    private fun pick(all: List<Triple<Device, MprisPlugin, Pair<Map<String, RemotePlayer>, Map<String, ByteArray>>>>, preferred: String?): Source? {
        val candidates = all.flatMap { (d, p, state) -> state.first.values.map { Triple(d, p, it) to state.second } }
        val chosen = candidates.firstOrNull { it.first.third.isPlaying && it.first.third.hasMedia }
            ?: candidates.firstOrNull { it.first.third.name == preferred && it.first.third.hasMedia }
            ?: all.firstNotNullOfOrNull { (d, p, state) -> p.activePlayer?.let { Triple(d, p, it) to state.second } }
            ?: return null
        val (d, p, player) = chosen.first
        return Source(d, p, player, chosen.second[player.artUrl], candidates.filter { it.first.third.hasMedia }.map { it.first.third.name })
    }

    val nowPlaying: StateFlow<NowPlaying?> = source
        .transformLatest { s ->
            if (s == null) {
                emit(null)
                return@transformLatest
            }
            val key = artKey(s.player)
            val cached = key?.let(cache::get)
            emit(NowPlaying(s.device.id, s.device.name, s.player, cached, s.players))
            if (cached == null && key != null) {
                val loaded = withContext(Dispatchers.IO) { loadArt(s) }
                if (loaded != null) {
                    cache.put(key, loaded)
                    emit(NowPlaying(s.device.id, s.device.name, s.player, loaded, s.players))
                }
            }
        }
        .stateIn(scope, SharingStarted.Eagerly, null)

    /** The plugin behind the current player, for controls. */
    fun control(block: suspend (MprisPlugin, String) -> Unit) {
        val np = nowPlaying.value ?: return
        scope.launch {
            val hubDevice = hub.value?.device(np.deviceId) ?: return@launch
            val plugin = hubDevice.plugin<MprisPlugin>(MprisPlugin.ID) ?: return@launch
            block(plugin, np.player.name)
        }
    }

    // ---- artwork ------------------------------------------------------------------------------

    private val cache = object : LruCache<String, Bitmap>(24 * 1024 * 1024) {
        override fun sizeOf(key: String, value: Bitmap) = value.allocationByteCount
    }

    private fun artKey(p: RemotePlayer): String? = youTubeId(p.url)?.let { "yt:$it" } ?: p.artUrl.takeIf { it.isNotBlank() }

    private fun loadArt(s: Source): Bitmap? {
        // YouTube in a browser: MPRIS art is a small page thumbnail, but the full-res frame is public.
        youTubeId(s.player.url)?.let { id ->
            for (variant in listOf("maxresdefault", "sddefault", "hqdefault")) {
                fetch("https://i.ytimg.com/vi/$id/$variant.jpg")?.let { bytes -> decode(bytes)?.takeIf { it.width > 120 }?.let { return it } }
            }
        }
        s.embeddedArt?.let { decode(it) }?.let { return it }
        val url = s.player.artUrl
        if (url.startsWith("https://") || url.startsWith("http://")) fetch(url)?.let { decode(it) }?.let { return it }
        return null
    }

    private fun fetch(url: String): ByteArray? = runCatching {
        val conn = URL(url).openConnection() as HttpURLConnection
        conn.connectTimeout = 4_000
        conn.readTimeout = 6_000
        conn.instanceFollowRedirects = true
        try {
            if (conn.responseCode != 200) return null
            conn.inputStream.use { it.readBytes() }
        } finally {
            conn.disconnect()
        }
    }.getOrNull()

    companion object {
        private const val MAX_ART_PX = 1024

        /** Decodes at no more than 1024px on the long side: crisp on a widget, cheap in memory. */
        fun decode(bytes: ByteArray): Bitmap? {
            val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
            BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
            if (bounds.outWidth <= 0) return null
            var sample = 1
            while (maxOf(bounds.outWidth, bounds.outHeight) / (sample * 2) >= MAX_ART_PX) sample *= 2
            return BitmapFactory.decodeByteArray(bytes, 0, bytes.size, BitmapFactory.Options().apply { inSampleSize = sample })
        }

        fun youTubeId(url: String): String? {
            if (url.isBlank()) return null
            val uri = runCatching { URI(url) }.getOrNull() ?: return null
            val host = uri.host?.removePrefix("www.")?.removePrefix("m.")?.removePrefix("music.") ?: return null
            return when (host) {
                "youtube.com" -> when {
                    uri.path == "/watch" -> uri.rawQuery?.split('&')?.firstOrNull { it.startsWith("v=") }?.removePrefix("v=")
                    uri.path.startsWith("/shorts/") || uri.path.startsWith("/live/") -> uri.path.split('/').getOrNull(2)
                    else -> null
                }
                "youtu.be" -> uri.path.trim('/').takeIf { it.isNotBlank() }
                else -> null
            }?.takeIf { it.matches(Regex("[A-Za-z0-9_-]{6,20}")) }
        }
    }
}
