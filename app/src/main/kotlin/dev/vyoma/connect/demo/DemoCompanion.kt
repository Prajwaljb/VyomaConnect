package dev.vyoma.connect.demo

import dev.vyoma.connect.protocol.DeviceIdentity
import dev.vyoma.connect.protocol.DeviceType
import dev.vyoma.connect.protocol.NetworkPacket
import dev.vyoma.connect.protocol.crypto.LocalCredentials
import dev.vyoma.connect.protocol.packet
import dev.vyoma.connect.protocol.plugin.builtin.CompanionPlugin
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.serialization.json.addJsonObject
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonArray
import kotlin.random.Random

/**
 * The simulated Vyoma companion beside [pc]: live stats, screenshots, webcam photos, handoff of
 * the browser video, a small home folder to browse, download from and upload into, and PC
 * controls (a monitor's brightness, two apps' volumes, power actions that do nothing).
 */
internal class DemoCompanion(credentials: LocalCredentials, private val pc: DemoPc) :
    SimulatedPeer(credentials, identity(credentials.deviceId, pc.id)) {

    private class Entry(val name: String, val size: Long = 0, val ageDays: Int = 1) {
        val isDir get() = size == 0L
    }

    private val booted = System.currentTimeMillis() - 3 * 3600_000L - 17 * 60_000L
    private var cpu = 14.0
    private var temp = 54.0
    private var leaseUntil = 0L
    private var streaming: Job? = null
    private var trigger = false

    /** Files the phone uploaded this session, per folder, shown on the next listing. */
    private val uploaded = mutableMapOf<String, MutableList<Entry>>()

    override fun onConnected(session: CoroutineScope) = Unit

    override fun onPacket(packet: NetworkPacket, session: CoroutineScope) {
        when (packet.type) {
            CompanionPlugin.STATS_REQUEST -> lease(session)
            CompanionPlugin.SCREENSHOT_REQUEST -> sendPayload(session, packet(CompanionPlugin.SCREENSHOT) { put("format", "png") }) { DemoArt.screenshot() }
            CompanionPlugin.CAMERA_REQUEST -> sendPayload(session, packet(CompanionPlugin.CAMERA) { put("format", "jpg") }) { DemoArt.photo() }
            CompanionPlugin.HANDOFF_REQUEST -> {
                if (packet.bool("pause") != false) pc.pauseVideo()
                send(packet(CompanionPlugin.HANDOFF) {
                    put("url", DemoPc.HANDOFF_URL)
                    put("title", pc.handoffTitle)
                    put("position", pc.handoffPosition())
                    put("exact", true)
                })
            }
            CompanionPlugin.FILES_LIST -> list(packet.string("path").orEmpty())
            CompanionPlugin.FILES_GET -> get(packet.string("path").orEmpty(), session)
            CompanionPlugin.FILES_PUT -> session.launch(Dispatchers.IO) { put(packet) }
            CompanionPlugin.PC_REQUEST -> sendControls()
            // A pretend game: pulling a trigger fires, and the pad kicks back (rumble).
            CompanionPlugin.GAMEPAD_STATE -> {
                val firing = (packet.int("rt") ?: 0) > 0 || (packet.int("lt") ?: 0) > 0
                if (firing && !trigger) send(packet(CompanionPlugin.GAMEPAD_RUMBLE) { put("strong", 42000); put("weak", 20000); put("ms", 140) })
                trigger = firing
            }
            CompanionPlugin.GAMEPAD_CLOSE -> trigger = false
            // A dusk wallpaper: deep blue sky over a warm horizon.
            CompanionPlugin.WALLPAPER_REQUEST -> send(packet(CompanionPlugin.WALLPAPER) {
                put("cols", 4)
                put("rows", 4)
                put("colors", kotlinx.serialization.json.JsonArray(DEMO_WALLPAPER.map { c ->
                    kotlinx.serialization.json.JsonArray(listOf(c shr 16 and 255, c shr 8 and 255, c and 255).map { kotlinx.serialization.json.JsonPrimitive(it) })
                }))
            })
            CompanionPlugin.PC_SET -> {
                packet.string("display")?.let { d -> packet.double("brightness")?.let { if (d == "display0") brightness = it.coerceIn(0.01, 1.0) } }
                packet.long("app")?.let { id ->
                    val i = apps.indexOfFirst { it.id == id }
                    if (i >= 0) apps[i] = apps[i].copy(
                        volume = packet.double("volume")?.coerceIn(0.0, 1.5) ?: apps[i].volume,
                        muted = packet.bool("muted") ?: apps[i].muted,
                    )
                }
                sendControls()
            }
            // Sends from the phone: nothing kept from before, so every file starts at 0.
            CompanionPlugin.XFER_BEGIN -> send(packet(CompanionPlugin.XFER_READY) { put("key", packet.string("key").orEmpty()); put("offset", 0) })
            CompanionPlugin.XFER_DATA -> session.launch(Dispatchers.IO) {
                val ok = receivePayload(packet) >= 0
                synchronized(this@DemoCompanion) {
                    send(packet(CompanionPlugin.XFER_DONE) {
                        put("key", packet.string("key").orEmpty())
                        if (ok) put("name", "file") else put("error", "The upload didn't arrive")
                    })
                }
            }
            // Nothing to put to sleep: the demo PC just says it did.
            CompanionPlugin.PC_POWER -> send(packet(CompanionPlugin.PC_POWERED) { put("action", packet.string("action").orEmpty()) })
        }
    }

    // ---- PC controls: one monitor, two apps playing --------------------------------------------

    private data class App(val id: Long, val name: String, val binary: String, val volume: Double, val muted: Boolean = false)

    private var brightness = 0.7
    private val apps = mutableListOf(App(41, "Firefox", "firefox", 0.85), App(57, "Music", "elisa", 0.6))

    private fun sendControls() = send(packet(CompanionPlugin.PC) {
        putJsonArray("displays") {
            addJsonObject { put("id", "display0"); put("label", "Studio Display"); put("internal", false); put("value", brightness) }
        }
        putJsonArray("apps") {
            for (a in apps) addJsonObject {
                put("id", a.id); put("name", a.name); put("binary", a.binary); put("volume", a.volume); put("muted", a.muted)
            }
        }
    })

    // ---- stats: streamed every 2 s for 12 s after each request, like the real companion ----------

    private fun lease(session: CoroutineScope) {
        leaseUntil = System.currentTimeMillis() + LEASE_MS
        if (streaming?.isActive == true) return
        streaming = session.launch {
            while (System.currentTimeMillis() < leaseUntil) {
                synchronized(this@DemoCompanion) { sendStats() }
                delay(STATS_EVERY_MS)
            }
        }
    }

    private fun sendStats() {
        // Wanders around a light workload and drifts back, rather than pinning at either end.
        cpu = (cpu + (15 - cpu) * 0.2 + Random.nextDouble(-5.0, 5.0)).coerceIn(3.0, 60.0)
        temp = (temp + (46 + cpu * 0.4 - temp) * 0.25 + Random.nextDouble(-0.5, 0.5)).coerceIn(40.0, 80.0)
        send(packet(CompanionPlugin.STATS) {
            put("cpu", cpu)
            put("memUsed", 9_400_000_000L + Random.nextLong(-120_000_000, 120_000_000))
            put("memTotal", 16_700_000_000L)
            put("cpuTemp", temp)
            put("gpu", (cpu * 0.6 + Random.nextDouble(0.0, 6.0)).toInt())
            put("uptime", (System.currentTimeMillis() - booted) / 1000)
            put("handoff", pc.handoffTitle)
        })
    }

    // ---- files --------------------------------------------------------------------------------

    private fun entries(path: String): List<Entry>? = TREE[path]?.let { it + uploaded[path].orEmpty() }

    private fun list(path: String) {
        val found = entries(path)
        val now = System.currentTimeMillis()
        send(packet(CompanionPlugin.FILES_LISTING) {
            put("path", path)
            if (found == null) put("error", "No such folder")
            putJsonArray("entries") {
                for (e in found.orEmpty().sortedWith(compareBy<Entry>({ !it.isDir }, { it.name.lowercase() }))) addJsonObject {
                    put("name", e.name)
                    put("dir", e.isDir)
                    put("size", sizeOf(path, e))
                    put("modified", now - e.ageDays * 86_400_000L)
                }
            }
        })
    }

    private fun get(path: String, session: CoroutineScope) {
        val folder = path.substringBeforeLast('/', "")
        val name = path.substringAfterLast('/')
        val entry = entries(folder)?.firstOrNull { it.name == name && !it.isDir }
        if (entry == null || kindOf(entry) == null) {
            send(packet(CompanionPlugin.FILES_FILE) {
                put("path", path); put("name", name)
                put("error", if (entry == null) "No such file" else "Too big for the demo; try a document or a picture")
            })
            return
        }
        // Pictures are drawn on the payload's IO thread, not while holding the peer's lock.
        sendPayload(session, packet(CompanionPlugin.FILES_FILE) { put("path", path); put("name", name) }) {
            synchronized(cache) { content(folder, entry)!! }
        }
    }

    private fun put(packet: NetworkPacket) {
        val folder = packet.string("path").orEmpty()
        val name = packet.string("name") ?: "upload"
        val size = receivePayload(packet)
        synchronized(this) {
            if (size >= 0 && TREE.containsKey(folder)) uploaded.getOrPut(folder) { mutableListOf() }.add(Entry(name, size.coerceAtLeast(1), 0))
            send(packet(CompanionPlugin.FILES_SAVED) {
                put("path", folder)
                put("name", name)
                if (size < 0) put("error", "The upload didn't arrive") else if (!TREE.containsKey(folder)) put("error", "No such folder")
            })
        }
    }

    private enum class Kind { TEXT, PICTURE }

    /** What a download can be: made-up text or a drawn picture; null for media too big to make up. */
    private fun kindOf(e: Entry) = when (e.name.substringAfterLast('.', "").lowercase()) {
        "md", "txt", "csv", "kt", "rs", "toml" -> Kind.TEXT
        "png", "jpg" -> Kind.PICTURE
        else -> null
    }

    /** Listed sizes: exact for text, a plausible figure for pictures (drawn only when downloaded). */
    private fun sizeOf(folder: String, e: Entry): Long = when {
        e.isDir -> 0
        e.size > 1 -> e.size
        kindOf(e) == Kind.TEXT -> textFor(e.name).length.toLong()
        else -> cache["$folder/${e.name}"]?.size?.toLong() ?: (180_000L + (e.name.hashCode() and 0x3ffff))
    }

    private fun content(folder: String, e: Entry): ByteArray? = when (kindOf(e)) {
        Kind.TEXT -> textFor(e.name).toByteArray()
        Kind.PICTURE -> cache.getOrPut("$folder/${e.name}") { DemoArt.cover(e.name.hashCode(), (e.name.hashCode() and 0xff) * 1.4f, 900) }
        null -> null
    }

    /** Drawn pictures, so a second download (and the listing after it) match the first. */
    private val cache = java.util.concurrent.ConcurrentHashMap<String, ByteArray>()

    private fun textFor(name: String) = when (name.substringAfterLast('.')) {
        "csv" -> "month,groceries,transport,fun\nJul,212,64,90\nAug,198,71,120\nSep,230,58,75\nOct,205,66,140\n"
        "kt" -> "fun main() {\n    println(\"Hello from the demo laptop\")\n}\n"
        "rs" -> "fn main() {\n    println!(\"Hello from the demo laptop\");\n}\n"
        "toml" -> "[package]\nname = \"orbit\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"
        else -> "# $name\n\nThis file lives on the simulated laptop in Vyoma's demo.\n\n- Downloads land in the phone's Downloads folder\n- Uploads show up in the folder you sent them to\n"
    }

    companion object {
        private const val LEASE_MS = 12_000L
        private const val STATS_EVERY_MS = 2_000L

        private val TREE: Map<String, List<Entry>> = mapOf(
            "" to listOf(Entry("Desktop"), Entry("Documents"), Entry("Downloads"), Entry("Music"), Entry("Pictures"), Entry("Projects"), Entry("Videos"), Entry("todo.md", 1, 0)),
            "Desktop" to listOf(Entry("ideas.txt", 1, 2), Entry("wallpaper.png", 1, 9)),
            "Documents" to listOf(Entry("Notes"), Entry("budget.csv", 1, 4), Entry("trip-plan.md", 1, 12), Entry("meeting-notes.txt", 1, 1)),
            "Documents/Notes" to listOf(Entry("reading-list.md", 1, 20), Entry("recipes.md", 1, 33)),
            "Downloads" to listOf(Entry("kdeconnect-tour.md", 1, 3), Entry("screenshot-2026-10-28.png", 1, 3), Entry("ubuntu-24.04-desktop.iso", 6_114_000_000, 40)),
            "Music" to listOf(Entry("Neon Coast"), Entry("Prism")),
            "Music/Neon Coast" to listOf(Entry("01 Night Drive.flac", 31_400_000, 60), Entry("02 Tail Lights.flac", 28_900_000, 60)),
            "Music/Prism" to listOf(Entry("01 Glass Horizon.flac", 27_100_000, 90)),
            "Pictures" to listOf(Entry("Screenshots"), Entry("hills.jpg", 1, 14), Entry("sunset.jpg", 1, 30), Entry("cat.png", 1, 5)),
            "Pictures/Screenshots" to listOf(Entry("desktop-1.png", 1, 2), Entry("desktop-2.png", 1, 1)),
            "Projects" to listOf(Entry("orbit"), Entry("README.md", 1, 6)),
            "Projects/orbit" to listOf(Entry("Cargo.toml", 1, 6), Entry("main.rs", 1, 6), Entry("App.kt", 1, 7)),
            "Videos" to listOf(Entry("himalaya-timelapse.mkv", 2_480_000_000, 15), Entry("birthday.mp4", 412_000_000, 120)),
        )

        fun identity(id: String, pcId: String) = DeviceIdentity(
            deviceId = id,
            deviceName = DemoPc.NAME,
            deviceType = DeviceType.LAPTOP,
            protocolVersion = DeviceIdentity.PROTOCOL_VERSION,
            incomingCapabilities = setOf(
                CompanionPlugin.STATS_REQUEST, CompanionPlugin.SCREENSHOT_REQUEST, CompanionPlugin.CAMERA_REQUEST, CompanionPlugin.HANDOFF_REQUEST,
                CompanionPlugin.FILES_LIST, CompanionPlugin.FILES_GET, CompanionPlugin.FILES_PUT, CompanionPlugin.WALLPAPER_REQUEST,
                CompanionPlugin.GAMEPAD_STATE, CompanionPlugin.GAMEPAD_CLOSE,
            ),
            outgoingCapabilities = setOf(
                CompanionPlugin.STATS, CompanionPlugin.SCREENSHOT, CompanionPlugin.CAMERA, CompanionPlugin.HANDOFF,
                CompanionPlugin.FILES_LISTING, CompanionPlugin.FILES_FILE, CompanionPlugin.FILES_SAVED, CompanionPlugin.WALLPAPER, CompanionPlugin.GAMEPAD_RUMBLE,
            ),
            companionOf = pcId,
        )
    }
}

private val DEMO_WALLPAPER = listOf(
    0x1B2A6B, 0x23307A, 0x2E3A8C, 0x3A3F95,
    0x2C4C9C, 0x4A5BB0, 0x6A5FB0, 0x8C5E9E,
    0x3E7FB8, 0x9C7AA8, 0xE07A6A, 0xF29A5C,
    0x14324F, 0x3A3A55, 0x6B4048, 0x8A4A3A,
)
