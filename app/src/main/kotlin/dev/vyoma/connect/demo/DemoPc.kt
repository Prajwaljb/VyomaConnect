package dev.vyoma.connect.demo

import dev.vyoma.connect.protocol.DeviceIdentity
import dev.vyoma.connect.protocol.DeviceType
import dev.vyoma.connect.protocol.NetworkPacket
import dev.vyoma.connect.protocol.PacketTypes
import dev.vyoma.connect.protocol.crypto.LocalCredentials
import dev.vyoma.connect.protocol.packet
import dev.vyoma.connect.protocol.putStrings
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.serialization.json.addJsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonArray
import kotlinx.serialization.json.putJsonObject

/**
 * The simulated laptop: answers what stock kdeconnectd would (battery, media players with
 * artwork, audio outputs, lock, commands, ping), with state that moves like a real one.
 */
internal class DemoPc(credentials: LocalCredentials) : SimulatedPeer(credentials, identity(credentials.deviceId)) {

    private data class Track(val title: String, val artist: String, val album: String, val lengthMs: Long, val hue: Float)

    private class Player(
        val name: String,
        val tracks: List<Track>,
        var index: Int = 0,
        var playing: Boolean = false,
        var positionMs: Long = 0,
        var since: Long = System.currentTimeMillis(),
        var volume: Int = 70,
        val url: String = "",
    ) {
        val track get() = tracks[index]
        fun position(now: Long = System.currentTimeMillis()) =
            (if (playing) positionMs + (now - since) else positionMs).coerceIn(0, track.lengthMs)
        fun settle(now: Long = System.currentTimeMillis()) { positionMs = position(now); since = now }
    }

    private class Sink(val name: String, val description: String, var volume: Int, var muted: Boolean, var default: Boolean)

    private val music = Player(
        "elisa",
        listOf(
            Track("Night Drive", "Aurora Lane", "Neon Coast", 214_000, 205f),
            Track("Monsoon Letters", "Kavya Rao", "Rainfall Sessions", 242_000, 160f),
            Track("Glass Horizon", "The Lumen Collective", "Prism", 187_000, 280f),
            Track("Slow Orbit", "Hollow Pines", "Satellites", 199_000, 20f),
            Track("Paper Lanterns", "Mira and the Tides", "Harbour Lights", 176_000, 330f),
        ),
        playing = true, positionMs = 48_000,
    )
    private val video = Player(
        "firefox",
        listOf(Track("Himalayan timelapse in 4K", "Open Skies", "", 612_000, 190f)),
        positionMs = 131_000, volume = 100, url = HANDOFF_URL,
    )
    private val players = listOf(music, video)

    private val sinks = listOf(
        Sink("alsa_output.speakers", "Laptop speakers", 36_000, muted = false, default = true),
        Sink("bluez_output.headphones", "Wireless headphones", 28_000, muted = false, default = false),
        Sink("alsa_output.hdmi", "Monitor (HDMI)", 52_000, muted = false, default = false),
    )

    private var battery = 76
    private var locked = false
    private var advance: Job? = null

    /** For the companion: the browser video it can hand off, and where it is. */
    val handoffTitle: String get() = video.track.title
    fun handoffPosition(): Long = video.position()

    @Synchronized
    fun pauseVideo() {
        if (!video.playing) return
        video.settle(); video.playing = false
        sendState(video)
    }

    override fun onConnected(session: CoroutineScope) {
        sendBattery()
        send(packet(PacketTypes.MOUSEPAD_KEYBOARDSTATE) { put("state", true) })
        // Plugged in: creeps up a percent every couple of minutes, like a laptop on its charger.
        session.launch {
            while (true) {
                delay(120_000)
                synchronized(this@DemoPc) { if (battery < 100) { battery++; sendBattery() } }
            }
        }
        scheduleAdvance(session)
    }

    override fun onPacket(packet: NetworkPacket, session: CoroutineScope) {
        when (packet.type) {
            PacketTypes.BATTERY_REQUEST -> sendBattery()
            PacketTypes.PING -> send(packet(PacketTypes.PING) { put("message", "Hello from $NAME") })
            PacketTypes.LOCK_REQUEST -> {
                packet.bool("setLocked")?.let { locked = it }
                send(packet(PacketTypes.LOCK) { put("isLocked", locked) })
            }
            PacketTypes.RUNCOMMAND_REQUEST -> if (packet.bool("requestCommandList") == true) sendCommands()
            PacketTypes.SYSTEMVOLUME_REQUEST -> onVolume(packet)
            PacketTypes.MPRIS_REQUEST -> onMedia(packet, session)
            PacketTypes.SHARE_REQUEST -> if (packet.hasPayload) session.launch(kotlinx.coroutines.Dispatchers.IO) { receivePayload(packet) }
            // Logged so a test can see what reached the "PC".
            PacketTypes.PRESENTER, PacketTypes.MOUSEPAD_REQUEST -> android.util.Log.d("DemoPc", "${packet.type} ${packet.body}")
            // Clipboard, touchpad and find-my-PC need no answer; the clipboard is never stored.
        }
    }

    private fun sendBattery() = send(packet(PacketTypes.BATTERY) {
        put("currentCharge", battery)
        put("isCharging", battery < 100)
        put("thresholdEvent", 0)
    })

    private fun sendCommands() {
        val list = buildJsonObject {
            for ((key, name, cmd) in COMMANDS) putJsonObject(key) { put("name", name); put("command", cmd) }
        }
        send(packet(PacketTypes.RUNCOMMAND) { put("commandList", list.toString()) })
    }

    // ---- audio outputs ------------------------------------------------------------------------

    private fun onVolume(packet: NetworkPacket) {
        if (packet.bool("requestSinks") == true) return sendSinks()
        val sink = sinks.firstOrNull { it.name == packet.string("name") } ?: return
        packet.int("volume")?.let { sink.volume = it.coerceIn(0, MAX_VOLUME) }
        packet.bool("muted")?.let { sink.muted = it }
        if (packet.bool("enabled") == true) sinks.forEach { it.default = it === sink }
        send(packet(PacketTypes.SYSTEMVOLUME) {
            put("name", sink.name)
            put("volume", sink.volume)
            put("muted", sink.muted)
            put("enabled", sink.default)
        })
    }

    private fun sendSinks() = send(packet(PacketTypes.SYSTEMVOLUME) {
        putJsonArray("sinkList") {
            for (s in sinks) addJsonObject {
                put("name", s.name)
                put("description", s.description)
                put("volume", s.volume)
                put("maxVolume", MAX_VOLUME)
                put("muted", s.muted)
                put("enabled", s.default)
            }
        }
    })

    // ---- media --------------------------------------------------------------------------------

    private fun onMedia(packet: NetworkPacket, session: CoroutineScope) {
        if (packet.bool("requestPlayerList") == true) {
            send(packet(PacketTypes.MPRIS) {
                putStrings("playerList", players.map { it.name })
                put("supportAlbumArtPayload", true)
            })
            return
        }
        val player = players.firstOrNull { it.name == packet.string("player") } ?: return
        packet.string("albumArtUrl")?.takeIf { packet.string("action") == null && !packet.has("requestNowPlaying") }?.let { url ->
            val track = player.tracks.getOrNull(url.substringAfterLast('/').substringBefore('.').toIntOrNull() ?: -1) ?: return
            val seed = player.tracks.indexOf(track) + players.indexOf(player) * 10
            sendPayload(session, packet(PacketTypes.MPRIS) {
                put("player", player.name)
                put("albumArtUrl", url)
                put("transferringAlbumArt", true)
            }) { DemoArt.cover(seed, track.hue) }
            return
        }
        var changed = packet.bool("requestNowPlaying") == true || packet.bool("requestVolume") == true
        packet.string("action")?.let { action ->
            player.settle()
            when (action) {
                "PlayPause" -> player.playing = !player.playing
                "Play" -> player.playing = true
                "Pause", "Stop" -> player.playing = false
                "Next" -> skip(player, +1)
                "Previous" -> if (player.position() > 3_000) player.positionMs = 0 else skip(player, -1)
            }
            // One thing plays at a time, as with MPRIS on a desktop.
            if (player.playing) players.filter { it !== player && it.playing }.forEach { it.settle(); it.playing = false; sendState(it) }
            changed = true
        }
        packet.long("SetPosition")?.let { player.positionMs = it.coerceIn(0, player.track.lengthMs); player.since = System.currentTimeMillis(); changed = true }
        packet.long("Seek")?.let { player.settle(); player.positionMs = (player.positionMs + it / 1000).coerceIn(0, player.track.lengthMs); changed = true }
        packet.int("setVolume")?.let { player.volume = it.coerceIn(0, 100); changed = true }
        if (changed) {
            sendState(player)
            scheduleAdvance(session)
        }
    }

    private fun skip(player: Player, by: Int) {
        player.index = (player.index + by).mod(player.tracks.size)
        player.positionMs = 0
        player.since = System.currentTimeMillis()
    }

    /** Moves to the next track when the current one ends, without polling. */
    private fun scheduleAdvance(session: CoroutineScope) {
        advance?.cancel()
        if (!music.playing) return
        advance = session.launch {
            delay(music.track.lengthMs - music.position() + 300)
            synchronized(this@DemoPc) {
                skip(music, +1)
                sendState(music)
                scheduleAdvance(session)
            }
        }
    }

    private fun sendState(p: Player) = send(packet(PacketTypes.MPRIS) {
        put("player", p.name)
        put("title", p.track.title)
        put("artist", p.track.artist)
        put("album", p.track.album)
        put("isPlaying", p.playing)
        put("pos", p.position())
        put("length", p.track.lengthMs)
        put("volume", p.volume)
        put("canPlay", true)
        put("canPause", true)
        put("canGoNext", p.tracks.size > 1)
        put("canGoPrevious", p.tracks.size > 1)
        put("canSeek", true)
        // file:// art arrives as a payload, like a local player's cover on a real PC.
        put("albumArtUrl", "file:///demo/${p.name}/${p.index}.png")
        put("url", p.url)
    })

    companion object {
        const val NAME = "Demo Laptop"
        const val MAX_VOLUME = 65_536
        const val HANDOFF_URL = "https://kdeconnect.kde.org/"

        private val COMMANDS = listOf(
            Triple("lock_screen", "Lock screen", "loginctl lock-session"),
            Triple("night_light", "Night light", "kwriteconfig6 --group NightColor --key Active true"),
            Triple("dnd", "Do not disturb", "qdbus org.freedesktop.Notifications /org/freedesktop/Notifications Inhibit"),
            Triple("suspend", "Suspend", "systemctl suspend"),
            Triple("open_terminal", "Open terminal", "konsole"),
        )

        fun identity(id: String) = DeviceIdentity(
            deviceId = id,
            deviceName = NAME,
            deviceType = DeviceType.LAPTOP,
            protocolVersion = DeviceIdentity.PROTOCOL_VERSION,
            incomingCapabilities = setOf(
                PacketTypes.PING, PacketTypes.BATTERY, PacketTypes.BATTERY_REQUEST, PacketTypes.CLIPBOARD, PacketTypes.CLIPBOARD_CONNECT,
                PacketTypes.FIND_MY_PHONE_REQUEST, PacketTypes.LOCK_REQUEST, PacketTypes.MPRIS_REQUEST, PacketTypes.MOUSEPAD_REQUEST, PacketTypes.PRESENTER,
                PacketTypes.RUNCOMMAND_REQUEST, PacketTypes.SYSTEMVOLUME_REQUEST, PacketTypes.SHARE_REQUEST,
            ),
            outgoingCapabilities = setOf(
                PacketTypes.PING, PacketTypes.BATTERY, PacketTypes.CLIPBOARD, PacketTypes.CLIPBOARD_CONNECT, PacketTypes.FIND_MY_PHONE_REQUEST,
                PacketTypes.LOCK, PacketTypes.MPRIS, PacketTypes.MOUSEPAD_KEYBOARDSTATE, PacketTypes.RUNCOMMAND, PacketTypes.SYSTEMVOLUME,
                PacketTypes.SHARE_REQUEST,
            ),
        )
    }
}
