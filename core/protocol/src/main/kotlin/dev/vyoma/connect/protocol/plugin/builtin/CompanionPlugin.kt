package dev.vyoma.connect.protocol.plugin.builtin

import dev.vyoma.connect.protocol.NetworkPacket
import dev.vyoma.connect.protocol.packet
import dev.vyoma.connect.protocol.plugin.Plugin
import dev.vyoma.connect.protocol.plugin.PluginContext
import dev.vyoma.connect.protocol.plugin.PluginSpec
import dev.vyoma.connect.protocol.VLog
import dev.vyoma.connect.protocol.link.PayloadSource
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.selects.select
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.doubleOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.longOrNull
import kotlinx.serialization.json.put
import dev.vyoma.connect.protocol.link.copyWithProgress
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/** A PC's vitals as the companion reports them. Missing sensors are null. */
data class PcStats(
    val cpu: Float,
    val memUsed: Long,
    val memTotal: Long,
    val cpuTemp: Float?,
    val gpu: Int?,
    val uptimeSeconds: Long?,
    /** Title of a web video the PC could hand off right now; null when nothing qualifies (mpv, local files). */
    val handoff: String? = null,
    /** False when no camera can see (the laptop's only one is behind its closed lid); null: not said. */
    val camera: Boolean? = null,
    /** With [camera] false: true when it's only a closed lid in the way, false when the PC has no camera. */
    val cameraLid: Boolean? = null,
    /** On its charger, whether or not the battery is taking charge (a charge limit holds it); null: not said. */
    val ac: Boolean? = null,
)

/**
 * The phone's game controller as the PC's virtual pad sees it: [buttons] a bitmask of
 * [GamepadButton]s, sticks -32768..32767 (y down), triggers 0..255, [kind] the pad the PC makes,
 * [mx]/[my] running totals of the gyro mouse's pixels (wrapping; the PC moves by the difference).
 */
data class GamepadState(
    val buttons: Int = 0,
    val lx: Int = 0,
    val ly: Int = 0,
    val rx: Int = 0,
    val ry: Int = 0,
    val lt: Int = 0,
    val rt: Int = 0,
    val kind: PadKind = PadKind.XBOX,
    val mx: Int = 0,
    val my: Int = 0,
)

private const val ECHO_EVERY = 32

/**
 * Spots a dead adb channel. Writes keep succeeding for over a second after the cable is pulled, until
 * adbd notices, so the only early sign is the PC's echoes stopping. Armed by the first echo: until then
 * the PC may just still be getting ready for the channel.
 */
internal class CableWatch(private val silentMs: Long = 250L) {
    private val armed = java.util.concurrent.atomic.AtomicBoolean(false)
    private val waitingSince = java.util.concurrent.atomic.AtomicLong(0L)

    fun sent(now: Long) { waitingSince.compareAndSet(0L, now) }

    fun echoed() {
        armed.set(true)
        waitingSince.set(0L)
    }

    fun silent(now: Long): Boolean {
        val since = waitingSince.get()
        return armed.get() && since != 0L && now - since > silentMs
    }
}

/** The latest timing of a route, for PC details' connection checks. */
data class TripReport(val medianMs: Long, val worstMs: Long)

/** Times frames the PC echoed, per route, for the log. Echoes come on their own coroutines. */
internal class RoundTrips(private val what: String = "controller", private val onReport: (TripReport) -> Unit = {}) {
    private val sent = java.util.concurrent.ConcurrentHashMap<Int, Long>()
    private val samples = mutableListOf<Long>()

    fun sending(seq: Int) {
        if (seq % ECHO_EVERY != 0) return
        if (sent.size > 64) sent.clear() // Lost echoes.
        sent[seq] = System.nanoTime()
    }

    fun echoed(seq: Int) {
        val at = sent.remove(seq) ?: return
        synchronized(samples) { samples += (System.nanoTime() - at) / 1_000_000 }
    }

    /** A frame sent on the old route says nothing about the new one, however late its echo comes. */
    fun moved() = sent.clear()

    /** Logs and clears what's been timed, returning how many samples that was. */
    fun report(route: Any?): Int {
        val all = synchronized(samples) { samples.sorted().also { samples.clear() } }
        if (all.isEmpty()) return 0
        VLog.i(
            "CompanionPlugin",
            "$what round trip over ${route ?: "the link"}: median ${all[all.size / 2]} ms, " +
                "95% under ${all[all.size * 95 / 100]} ms, worst ${all.last()} ms (${all.size} samples)",
        )
        onReport(TripReport(all[all.size / 2], all.last()))
        return all.size
    }
}

/** How the controller's states reach the PC beside the link. */
enum class GamepadRoute { WIFI, BLUETOOTH, USB }

/** The pad games on the PC see. */
enum class PadKind(val wire: String) { XBOX("xbox"), DUALSENSE("dualsense") }

/** Bits of [GamepadState.buttons]; the companion's gamepad.rs uses the same order. */
enum class GamepadButton { A, B, X, Y, LB, RB, BACK, START, GUIDE, LS, RS, UP, DOWN, LEFT, RIGHT;
    val bit: Int get() = 1 shl ordinal
}

/** A game's rumble: strengths 0..65535 (the strong low motor, the weak high one); [ms] 0 = until the next. */
data class Rumble(val strong: Int, val weak: Int, val ms: Int)

/** Where a screenshot or camera photo request is. [Done.location] is something the UI can open. */
sealed interface CaptureState {
    data object Idle : CaptureState
    data object Capturing : CaptureState
    data class Done(val location: String, val mime: String, val at: Long) : CaptureState
    data class Failed(val reason: String, val at: Long) : CaptureState
}

/**
 * The PC's wallpaper as a [cols] x [rows] grid of colours (0xRRGGBB, row-major from the top left),
 * for the phone's background: stretched smoothly, it's the wallpaper's light, blurred.
 */
data class PcWallpaper(val cols: Int, val rows: Int, val colors: List<Int>)

/**
 * What the PC is, for its details page: each part null (or empty) when the PC can't say. Sizes in
 * bytes; [displays] built-in first, at their native size.
 */
data class PcSpecs(
    val model: String?,
    val os: String?,
    val kernel: String?,
    val desktop: String?,
    val session: String?,
    val cpu: String?,
    val cores: Int?,
    val threads: Int?,
    val gpus: List<String>,
    val memTotal: Long?,
    val diskTotal: Long?,
    val diskFree: Long?,
    val displays: List<Display>,
) {
    data class Display(val width: Int, val height: Int, val internal: Boolean)

    companion object {
        /** From a `vyoma.specs` body (also how the app keeps the last one); null if it says nothing. */
        fun parse(body: JsonObject): PcSpecs? {
            if (body["error"] != null) return null
            fun str(k: String) = (body[k] as? JsonPrimitive)?.takeIf { it.isString }?.content?.trim()?.takeIf { it.isNotEmpty() }?.take(120)
            fun num(k: String) = (body[k] as? JsonPrimitive)?.longOrNull?.takeIf { it > 0 }
            val gpus = (body["gpus"] as? JsonArray).orEmpty().mapNotNull { (it as? JsonPrimitive)?.takeIf { p -> p.isString }?.content?.trim()?.takeIf { s -> s.isNotEmpty() }?.take(120) }.take(4)
            val displays = (body["displays"] as? JsonArray).orEmpty().mapNotNull { e ->
                val o = e as? JsonObject ?: return@mapNotNull null
                val w = (o["w"] as? JsonPrimitive)?.intOrNull ?: return@mapNotNull null
                val h = (o["h"] as? JsonPrimitive)?.intOrNull ?: return@mapNotNull null
                if (w <= 0 || h <= 0) null else Display(w, h, (o["internal"] as? JsonPrimitive)?.booleanOrNull == true)
            }.take(6)
            val specs = PcSpecs(
                str("model"), str("os"), str("kernel"), str("desktop"), str("session"), str("cpu"),
                num("cores")?.toInt(), num("threads")?.toInt(), gpus, num("memTotal"), num("diskTotal"),
                (body["diskFree"] as? JsonPrimitive)?.longOrNull?.coerceAtLeast(0), displays,
            )
            return specs.takeIf { it.os != null || it.cpu != null || it.memTotal != null }
        }
    }
}

/** One entry of a PC folder. Sizes are 0 for folders; [modified] is epoch millis. */
data class PcFile(val name: String, val isDir: Boolean, val size: Long, val modified: Long)

/** A PC folder as last listed. [path] is relative to the PC user's home ("" is home). */
data class PcFolder(
    val path: String,
    val entries: List<PcFile> = emptyList(),
    val loading: Boolean = true,
    val error: String? = null,
    val truncated: Boolean = false,
)

/**
 * Where the PC was in what it's playing, for carrying on here. [positionMs] is null when the PC
 * couldn't tell; [exact] is false when it estimated it (Firefox reports no position).
 */
data class PcHandoff(val url: String, val title: String, val positionMs: Long?, val exact: Boolean)

/** A screen whose brightness the PC can set (laptop panel, or a monitor over DDC). [level] is 0..1. */
data class PcDisplay(val id: String, val label: String, val internal: Boolean, val level: Float)

/** Something playing sound on the PC. [volume] is 0..1, past 1 if it was boosted on the PC. */
data class PcApp(val id: Long, val name: String, val binary: String?, val title: String?, val volume: Float, val muted: Boolean)

data class PcControls(val displays: List<PcDisplay> = emptyList(), val apps: List<PcApp> = emptyList())

/** What the PC can be asked to do with itself. All go through Plasma, as from its own menu. */
/**
 * What the PC does by itself once this phone has been gone [afterSeconds] (on top of the ~30 s it
 * takes to notice), from the phone's "PC goes away" rules; with [undo], put back when it returns.
 */
data class AwayRules(val lock: AwayAction? = null, val pause: AwayAction? = null, val mute: AwayAction? = null)

/** One thing the PC does once the phone has gone: how long after, and whether it's put back on return. */
data class AwayAction(val afterSeconds: Int, val undo: Boolean)

enum class PcPower(val wire: String) { SLEEP("sleep"), RESTART("restart"), SHUTDOWN("shutdown"), LOGOUT("logout") }

/** How one attempt at a resumable send ended; [Failed.retry] says whether trying again can help. */
sealed interface XferResult {
    data class Saved(val name: String) : XferResult
    data class Failed(val reason: String, val retry: Boolean) : XferResult
}

sealed interface PcFileEvent {
    /** [temporary]: fetched only to look at (see [CompanionPlugin.download]), not saved to the user's files. */
    data class Saved(val name: String, val location: String, val temporary: Boolean = false) : PcFileEvent
    data class Failed(val name: String, val reason: String) : PcFileEvent
    /** Uploaded into [folder]; [name] is what the PC saved it as (it never overwrites). */
    data class Uploaded(val name: String, val folder: String) : PcFileEvent
}

/**
 * Talks to the Vyoma companion (desktop/companion). Stats are a lease: the companion streams
 * every 2 s for 12 s after each [requestStats], so callers renew while something is on screen
 * and the PC goes quiet as soon as nobody looks.
 */
class CompanionPlugin(private val ctx: PluginContext, private val sink: ShareSink) : Plugin {
    private val _stats = MutableStateFlow<PcStats?>(null)
    val stats: StateFlow<PcStats?> = _stats.asStateFlow()


    private val _folder = MutableStateFlow<PcFolder?>(null)
    val folder: StateFlow<PcFolder?> = _folder.asStateFlow()
    /** Every folder listed this session, so a folder page shows at once (going back to it, or swiping back over it). */
    private val listed = java.util.concurrent.ConcurrentHashMap<String, PcFolder>()
    fun cached(path: String): PcFolder? = listed[path]

    /** PC paths being downloaded right now. */
    private val _fetching = MutableStateFlow<Set<String>>(emptySet())
    val fetching: StateFlow<Set<String>> = _fetching.asStateFlow()
    private val _fileEvents = MutableSharedFlow<PcFileEvent>(extraBufferCapacity = 8)
    val fileEvents: SharedFlow<PcFileEvent> = _fileEvents.asSharedFlow()

    suspend fun requestStats() = ctx.send(packet(STATS_REQUEST))

    private val _controls = MutableStateFlow<PcControls?>(null)
    /** Brightness and app volumes as last reported; null until the first [requestControls]. */
    val controls: StateFlow<PcControls?> = _controls.asStateFlow()

    suspend fun requestControls() = ctx.send(packet(PC_REQUEST))

    private val _bluetoothAddress = MutableStateFlow<String?>(null)
    /** This PC's Bluetooth address, for when Wi-Fi is gone (the app's BluetoothTunnel). */
    val bluetoothAddress: StateFlow<String?> = _bluetoothAddress.asStateFlow()

    private val _wallpaper = MutableStateFlow<PcWallpaper?>(null)
    /** The PC's wallpaper colours, asked for on every link (it may have changed since). */
    val wallpaper: StateFlow<PcWallpaper?> = _wallpaper.asStateFlow()

    suspend fun requestWallpaper() = ctx.send(packet(WALLPAPER_REQUEST))

    private val _specs = MutableStateFlow<Pair<PcSpecs, JsonObject>?>(null)
    /** What the PC is, with the body it came in (for keeping); asked for on every link. */
    val specs: StateFlow<Pair<PcSpecs, JsonObject>?> = _specs.asStateFlow()

    suspend fun requestSpecs() = ctx.send(packet(SPECS_REQUEST))

    private fun receiveWallpaper(packet: NetworkPacket) {
        val cols = packet.int("cols") ?: return
        val rows = packet.int("rows") ?: return
        val colors = (packet.body["colors"] as? JsonArray).orEmpty().mapNotNull { e ->
            val c = (e as? JsonArray)?.mapNotNull { it.jsonPrimitive.intOrNull } ?: return@mapNotNull null
            if (c.size < 3) null else (c[0].coerceIn(0, 255) shl 16) or (c[1].coerceIn(0, 255) shl 8) or c[2].coerceIn(0, 255)
        }
        if (cols > 0 && rows > 0 && colors.size == cols * rows) _wallpaper.value = PcWallpaper(cols, rows, colors)
    }

    /** Changes waiting to go, by what they change: a drag sends its newest value ~16 times a second. */
    private val pendingSets = LinkedHashMap<String, JsonObject>()
    private var setSender: Job? = null

    fun setBrightness(display: String, level: Float) = queueSet("d:$display", buildJsonObject { put("display", display); put("brightness", level.toDouble()) })
    fun setAppVolume(app: Long, volume: Float) = queueSet("v:$app", buildJsonObject { put("app", app); put("volume", volume.toDouble()) })
    fun setAppMuted(app: Long, muted: Boolean) = queueSet("m:$app", buildJsonObject { put("app", app); put("muted", muted) })

    private fun queueSet(key: String, body: JsonObject) {
        synchronized(pendingSets) {
            pendingSets.remove(key)
            pendingSets[key] = body
            if (setSender?.isActive == true) return
            setSender = ctx.scope.launch {
                while (true) {
                    val next = synchronized(pendingSets) {
                        // Stepping down in the same lock, or a set arriving now would wait for this one.
                        pendingSets.values.toList().also { pendingSets.clear(); if (it.isEmpty()) setSender = null }
                    }
                    if (next.isEmpty()) break
                    for (b in next) ctx.send(NetworkPacket(PC_SET, b))
                    delay(SET_INTERVAL_MS)
                }
            }
        }
    }

    /** Answers to resumable sends, by "ready:key" / "done:key". */
    private val xferReplies = java.util.concurrent.ConcurrentHashMap<String, CompletableDeferred<NetworkPacket>>()

    /**
     * One attempt at sending [file] into the PC's downloads (inside [folder], "Photos/2024", when
     * it's part of a sent folder). The PC says how much it already has under [key] from an attempt
     * that broke off, and only the rest is sent. [onProgress] gets the bytes the PC has in total.
     */
    suspend fun sendResumable(file: OutgoingFile, key: String, folder: String?, batch: String?, onProgress: (Long) -> Unit): XferResult {
        val ready = CompletableDeferred<NetworkPacket>().also { xferReplies["ready:$key"] = it }
        val begun = ctx.send(packet(XFER_BEGIN) {
            put("key", key); put("name", file.name); put("size", file.size)
            folder?.let { put("folder", it) }
            batch?.let { put("batch", it) }
        })
        val answer = if (begun) withTimeoutOrNull(XFER_REPLY_MS) { ready.await() } else null
        xferReplies.remove("ready:$key")
        answer ?: return XferResult.Failed("The PC didn't answer", retry = true)
        if (answer.bool(LOST) == true) return XferResult.Failed("The PC went away", retry = true)
        answer.string("error")?.let { return XferResult.Failed(it, retry = false) }
        val offset = (answer.long("offset") ?: 0).coerceIn(0, file.size)
        onProgress(offset)

        val done = CompletableDeferred<NetworkPacket>().also { xferReplies["done:$key"] = it }
        val result = coroutineScope {
            val sending = async {
                runCatching {
                    ctx.device.sendWithPayload(
                        packet(XFER_DATA) { put("key", key); put("offset", offset) },
                        PayloadSource(file.size - offset) { file.open().also { it.skipFully(offset) } },
                        timeoutMs = 60_000,
                    ) { n -> onProgress(offset + n) }
                }.onFailure { VLog.w(TAG, "send of ${file.name} broke off: ${it.message}") }.getOrDefault(false)
            }
            // Whichever comes first: the PC's answer (or the companion going away, see onStop)
            // ends a send still stuck on a dead socket; a finished send waits for the answer.
            val first = select<NetworkPacket?> {
                done.onAwait { it }
                sending.onAwait { null }
            }
            if (first != null) {
                sending.cancel()
                first
            } else {
                withTimeoutOrNull(if (sending.await()) XFER_SAVE_MS else XFER_REPLY_MS) { done.await() }
            }
        }
        xferReplies.remove("done:$key")
        result ?: return XferResult.Failed("The transfer was interrupted", retry = true)
        if (result.bool(LOST) == true) return XferResult.Failed("The PC went away", retry = true)
        result.string("error")?.let { return XferResult.Failed(it, retry = true) }
        return XferResult.Saved(result.string("name") ?: file.name)
    }

    /** A sent folder is complete: the PC shows one notification for it. */
    suspend fun endBatch(batch: String, count: Int) = ctx.send(packet(XFER_END) { put("batch", batch); put("count", count) })

    /** The companion went: sends waiting on it give up now and resume once it's back. */
    override fun onStop() = abandonReplies()

    /**
     * A companion has one link, so a new one means it restarted (or Wi-Fi changed under it) and the
     * old session's answers will never come; the device can stay reachable throughout, so no onStop.
     */
    override suspend fun onLinkAdded() {
        abandonReplies()
        sendAway()
        requestWallpaper()
        requestSpecs()
    }

    override suspend fun onStart() {
        sendAway()
        requestWallpaper()
        requestSpecs()
    }

    /** Tells the PC the current walk-away rules ([away]); sent on every link too, so a restarted companion knows them. */
    suspend fun sendAway() {
        val rules = away.value
        ctx.send(packet(AWAY) {
            fun action(name: String, a: AwayAction?) {
                if (a != null) put(name, buildJsonObject { put("after", a.afterSeconds); put("undo", a.undo) })
            }
            action("lock", rules.lock); action("pause", rules.pause); action("mute", rules.mute)
        })
    }

    private fun abandonReplies() {
        _fetching.value = emptySet()
        _folder.update { if (it?.loading == true) it.copy(loading = false, error = "The PC didn't answer") else it }
        for (key in xferReplies.keys.toList()) xferReplies.remove(key)?.complete(packet(XFER_DONE) { put(LOST, true) })
    }

    private fun receiveXfer(prefix: String, packet: NetworkPacket) {
        val key = packet.string("key") ?: return
        xferReplies.remove("$prefix:$key")?.complete(packet)
    }

    /** Turns the PC's screens back on and unlocks it (the stock unlock left a dark screen dark). */
    suspend fun wake() = ctx.send(packet(PC_WAKE))

    private var pendingPaste: CompletableDeferred<String?>? = null
    private val _typingFailed = MutableSharedFlow<Unit>(extraBufferCapacity = 1, onBufferOverflow = kotlinx.coroutines.channels.BufferOverflow.DROP_OLDEST)
    /** Text sent to the live screen didn't all arrive. */
    val typingFailed: SharedFlow<Unit> = _typingFailed.asSharedFlow()

    /**
     * Puts [text] on the PC's clipboard for a paste. True once it's there; the PC puts back what
     * was on it by itself. Only one at a time.
     */
    suspend fun preparePaste(text: String): Boolean {
        val reply = CompletableDeferred<String?>()
        pendingPaste = reply
        ctx.send(packet(PASTE) { put("text", text) })
        val answer = withTimeoutOrNull(PASTE_WAIT_MS) { reply.await() ?: "" }
        return answer == ""
    }

    private var pendingPower: CompletableDeferred<String?>? = null

    /**
     * Asks the PC to sleep, restart, shut down or log out. Plasma does it as from its own menu
     * (apps are asked to close). Fails with a message for the user if the PC refused.
     */
    suspend fun power(action: PcPower): Result<Unit> {
        val reply = CompletableDeferred<String?>()
        pendingPower = reply
        if (!ctx.send(packet(PC_POWER) { put("action", action.wire) })) return Result.failure(IllegalStateException("The PC isn't reachable"))
        // No answer usually means it already went (sleep can beat the reply out of the door).
        val error = withTimeoutOrNull(POWER_TIMEOUT_MS) { reply.await() }
        return if (error == null) Result.success(Unit) else Result.failure(IllegalStateException(error))
    }

    private fun receiveControls(packet: NetworkPacket) {
        fun JsonObject.str(k: String) = this[k]?.jsonPrimitive?.contentOrNull
        fun JsonObject.num(k: String) = this[k]?.jsonPrimitive?.doubleOrNull
        val displays = (packet.body["displays"] as? JsonArray).orEmpty().mapNotNull { e ->
            val o = e as? JsonObject ?: return@mapNotNull null
            PcDisplay(
                id = o.str("id") ?: return@mapNotNull null,
                label = o.str("label").orEmpty(),
                internal = o["internal"]?.jsonPrimitive?.booleanOrNull ?: false,
                level = o.num("value")?.toFloat() ?: return@mapNotNull null,
            )
        }
        val apps = (packet.body["apps"] as? JsonArray).orEmpty().mapNotNull { e ->
            val o = e as? JsonObject ?: return@mapNotNull null
            PcApp(
                id = o["id"]?.jsonPrimitive?.longOrNull ?: return@mapNotNull null,
                name = o.str("name") ?: "App",
                binary = o.str("binary"),
                title = o.str("title"),
                volume = o.num("volume")?.toFloat() ?: 1f,
                muted = o["muted"]?.jsonPrimitive?.booleanOrNull ?: false,
            )
        }
        _controls.value = PcControls(displays, apps)
    }

    /** Lists a home-relative PC folder; [folder] shows it loading, then the result. */
    suspend fun listFiles(path: String) {
        val previous = _folder.value
        // Keep the old entries on a refresh of the same folder so the list doesn't flash empty.
        _folder.value = if (previous?.path == path) previous.copy(loading = true, error = null)
            else listed[path]?.copy(loading = true) ?: PcFolder(path)
        if (!ctx.send(packet(FILES_LIST) { put("path", path) })) {
            _folder.update { if (it?.path == path) it.copy(loading = false, error = "The PC isn't reachable") else it }
        }
    }

    /** Paths fetched only to look at; see [download]. */
    private val viewing = java.util.concurrent.ConcurrentHashMap.newKeySet<String>()

    /**
     * Fetches a PC file. [keep]: into the phone's Downloads, with progress like any received file;
     * otherwise only to look at, somewhere temporary that clears itself ([ShareSink.createTemporary]).
     */
    suspend fun download(path: String, keep: Boolean = true) {
        if (path in _fetching.value) return
        if (keep) viewing.remove(path) else viewing.add(path)
        _fetching.update { it + path }
        if (!ctx.send(packet(FILES_GET) { put("path", path) })) {
            _fetching.update { it - path }
            _fileEvents.tryEmit(PcFileEvent.Failed(path.substringAfterLast('/'), "The PC isn't reachable"))
        }
    }

    /** The companion's answer to the upload in flight: null error means saved. */
    private class Pending(val folder: String, val name: String, val reply: CompletableDeferred<Pair<String, String?>>)
    @Volatile private var pending: Pending? = null
    private val uploadLock = Mutex()

    /** Uploads into a home-relative PC folder, one file at a time, with the usual transfer progress. */
    suspend fun upload(folder: String, files: List<OutgoingFile>) = withContext(Dispatchers.IO) {
        uploadLock.withLock {
            for (file in files) {
                val id = Transfers.start(ctx.device, file.name, incoming = false, total = file.size)
                val reply = CompletableDeferred<Pair<String, String?>>()
                pending = Pending(folder, file.name, reply)
                val answer = coroutineScope {
                    val send = async {
                        runCatching {
                            ctx.device.sendWithPayload(
                                packet(FILES_PUT) { put("path", folder); put("name", file.name) },
                                PayloadSource(file.size, file.open), timeoutMs = 60_000,
                            ) { done -> Transfers.progress(id, done) }
                        }.onFailure { VLog.w(TAG, "upload failed: ${it.message}") }.getOrDefault(false)
                    }
                    // If the PC refuses up front (bad folder, odd name) it never connects; stop waiting.
                    val refused = launch { if (reply.await().second != null) send.cancel() }
                    val sent = runCatching { send.await() }.getOrDefault(false)
                    val answer = if (sent || reply.isCompleted) withTimeoutOrNull(15_000) { reply.await() } else null
                    refused.cancel()
                    answer
                }
                pending = null
                val error = when {
                    answer == null -> "The transfer was interrupted"
                    else -> answer.second
                }
                Transfers.finish(id, ok = error == null)
                _fileEvents.tryEmit(if (error == null) PcFileEvent.Uploaded(answer!!.first, folder) else PcFileEvent.Failed(file.name, error))
            }
            if (_folder.value?.path == folder) listFiles(folder)
        }
    }

    private fun receiveSaved(packet: NetworkPacket) {
        val p = pending ?: return
        if (packet.string("path") != p.folder) return
        p.reply.complete((packet.string("name") ?: p.name) to packet.string("error"))
    }

    private fun receiveListing(packet: NetworkPacket) {
        val path = packet.string("path") ?: return
        val entries = (packet.body["entries"] as? JsonArray).orEmpty().mapNotNull { e ->
            val o = e as? JsonObject ?: return@mapNotNull null
            PcFile(
                name = o["name"]?.jsonPrimitive?.contentOrNull ?: return@mapNotNull null,
                isDir = o["dir"]?.jsonPrimitive?.booleanOrNull ?: false,
                size = o["size"]?.jsonPrimitive?.longOrNull ?: 0,
                modified = o["modified"]?.jsonPrimitive?.longOrNull ?: 0,
            )
        }
        val result = PcFolder(path, entries, loading = false, error = packet.string("error"), truncated = packet.bool("truncated") == true)
        if (result.error == null) listed[path] = result
        _folder.update { current ->
            if (current != null && current.path != path) return@update current // the user has moved on
            result
        }
    }

    private fun receiveFile(packet: NetworkPacket) {
        val path = packet.string("path").orEmpty()
        val name = SharePlugin.sanitizeFileName(packet.string("name") ?: path.substringAfterLast('/').ifEmpty { "file" })
        fun done(event: PcFileEvent) {
            _fetching.update { it - path }
            _fileEvents.tryEmit(event)
        }
        packet.string("error")?.let { return done(PcFileEvent.Failed(name, it)) }
        if (!packet.hasPayload) return done(PcFileEvent.Failed(name, "The PC sent nothing"))
        val view = viewing.remove(path)
        ctx.scope.launch(Dispatchers.IO) {
            val size = packet.payloadSize
            // A file only being looked at isn't a received file: no transfer entry or notification.
            val id = if (view) null else Transfers.start(ctx.device, name, incoming = true, total = size)
            val target = if (view) sink.createTemporary(ctx.device, name, size) else sink.createFile(ctx.device, name, size, null)
            if (target == null) {
                id?.let { Transfers.finish(it, ok = false) }
                return@launch done(PcFileEvent.Failed(name, "Couldn't save it"))
            }
            try {
                val input = ctx.device.openPayload(packet) ?: error("no payload")
                input.use { it.copyWithProgress(target.output) { n -> id?.let { Transfers.progress(it, n) } } }
                target.output.close()
                val location = target.complete() ?: error("not saved")
                id?.let { Transfers.finish(it, ok = true, location = location) }
                done(PcFileEvent.Saved(name, location, temporary = view))
            } catch (e: Exception) {
                VLog.w(TAG, "file transfer failed: ${e.message}")
                target.abort()
                id?.let { Transfers.finish(it, ok = false) }
                done(PcFileEvent.Failed(name, "The transfer was interrupted"))
            }
        }
    }

    /**
     * One kind of picture the PC takes on request (a screenshot, a camera photo): asks for it,
     * saves the payload to Pictures, and follows it in [state] to Done or Failed, with a watchdog
     * so the UI never waits on a spinner forever.
     */
    inner class Capture internal constructor(private val request: String, private val fileLabel: String) {
        private val _state = MutableStateFlow<CaptureState>(CaptureState.Idle)
        val state: StateFlow<CaptureState> = _state.asStateFlow()
        private var watchdog: Job? = null

        suspend fun request() {
            if (_state.value == CaptureState.Capturing) return
            _state.value = CaptureState.Capturing
            if (!ctx.send(packet(request))) return fail("The PC isn't reachable")
            watchdog?.cancel()
            watchdog = ctx.scope.launch {
                delay(CAPTURE_TIMEOUT_MS)
                if (_state.value == CaptureState.Capturing) fail("The PC didn't answer")
            }
        }

        private fun fail(reason: String) {
            watchdog?.cancel()
            _state.value = CaptureState.Failed(reason, System.currentTimeMillis())
        }

        internal fun receive(packet: NetworkPacket) {
            packet.string("error")?.let { return fail(it) }
            if (!packet.hasPayload) return fail("The PC sent no image")
            watchdog?.cancel() // it answered: on slow Wi-Fi the transfer can take longer, and has timeouts of its own
            ctx.scope.launch(Dispatchers.IO) {
                val ext = packet.string("format")?.takeIf { it in IMAGE_FORMATS } ?: "png"
                val name = "$fileLabel " + SimpleDateFormat("yyyy-MM-dd HH.mm.ss", Locale.US).format(Date()) + ".$ext"
                val target = sink.createPicture(ctx.device, name, packet.payloadSize) ?: return@launch fail("Couldn't save the image")
                try {
                    val input = ctx.device.openPayload(packet) ?: error("no payload")
                    input.use { it.copyTo(target.output, 256 * 1024) }
                    target.output.close()
                    val location = target.complete() ?: error("not saved")
                    watchdog?.cancel()
                    _state.value = CaptureState.Done(location, if (ext == "jpg") "image/jpeg" else "image/$ext", System.currentTimeMillis())
                } catch (e: Exception) {
                    VLog.w(TAG, "$fileLabel transfer failed: ${e.message}")
                    target.abort()
                    fail("The transfer was interrupted")
                }
            }
        }
    }

    private var pendingHandoff: CompletableDeferred<Result<PcHandoff>>? = null

    /**
     * Asks the PC what it's playing and where (and to pause it, so it doesn't play on over the
     * phone). Fails with a message for the user if nothing with a link is playing.
     */
    suspend fun handoff(pause: Boolean = true): Result<PcHandoff> {
        val reply = CompletableDeferred<Result<PcHandoff>>()
        pendingHandoff = reply
        if (!ctx.send(packet(HANDOFF_REQUEST) { put("pause", pause) })) return Result.failure(IllegalStateException("The PC isn't reachable"))
        return withTimeoutOrNull(HANDOFF_TIMEOUT_MS) { reply.await() } ?: Result.failure(IllegalStateException("The PC didn't answer"))
    }

    private fun receiveHandoff(packet: NetworkPacket) {
        val reply = pendingHandoff ?: return
        pendingHandoff = null
        val error = packet.string("error")
        val url = packet.string("url")
        reply.complete(
            if (error != null || url == null) Result.failure(IllegalStateException(error ?: "The PC sent no link"))
            else Result.success(PcHandoff(url, packet.string("title").orEmpty(), packet.long("position"), packet.bool("exact") == true)),
        )
    }

    /** The PC's screen as a live stream: the open payload and the picture size. Close [video] to stop. */
    /** [control]: the PC lets the phone's touches work its pointer ([screenInput]). */
    class ScreenStream(val width: Int, val height: Int, val video: java.io.InputStream, val control: Boolean = false)

    private var pendingScreen: CompletableDeferred<NetworkPacket>? = null

    /**
     * Starts a live cast of the PC's screen. The first time, Plasma asks on the PC which screen to
     * share, so this waits up to a minute for the answer. [maxFps]: the most frames a second this
     * phone shows and decodes (the PC sends up to 120, 60 if not told).
     */
    suspend fun openScreen(maxFps: Int? = null): Result<ScreenStream> {
        val reply = CompletableDeferred<NetworkPacket>()
        pendingScreen = reply
        if (!ctx.send(packet(SCREEN_REQUEST) { maxFps?.let { put("maxFps", it) } })) return Result.failure(IllegalStateException("The PC isn't reachable"))
        val answer = withTimeoutOrNull(SCREEN_TIMEOUT_MS) { reply.await() }
            ?: return Result.failure(IllegalStateException("The PC didn't answer. Is screen sharing waiting for you there?"))
        answer.string("error")?.let { return Result.failure(IllegalStateException(it)) }
        if (!answer.hasPayload) return Result.failure(IllegalStateException("The PC sent no video"))
        return runCatching {
            val video = withContext(Dispatchers.IO) { ctx.device.openPayload(answer) } ?: error("Couldn't open the video stream")
            ScreenStream(answer.int("width") ?: 0, answer.int("height") ?: 0, video, answer.bool("control") ?: false)
        }
    }

    /** Ends the cast on the PC (closing the stream does too; this is quicker and says so). */
    suspend fun stopScreen() = ctx.send(packet(SCREEN_STOP))

    /**
     * A touch on the PC's screen as the phone shows it: [x], [y] 0..1 across the picture. Kinds:
     * "click", "right", "down" / "move" / "up" (a left-button drag), "scroll" ([dx], [dy] in pixels),
     * "scrollend"; and typing: "text" ([text]) and "key" ([key]: BackSpace, Return, Escape, Tab,
     * Delete, Home, End, Left/Right/Up/Down, Super, or one character, with [mods] ctrl/alt/super/shift).
     */
    /**
     * Touches for the PC's pointer as a live stream instead of packets: one JSON line per touch, as
     * [screenInput]'s body. A payload rides the USB cable when it's plugged in (the link itself is
     * Wi-Fi, whose 50-250 ms hiccups made drags stutter, 2026-11-04). Suspends until it ends.
     */
    suspend fun screenTouches(lines: java.io.InputStream): Boolean =
        ctx.device.sendWithPayload(packet(SCREEN_TOUCH), PayloadSource(-1) { lines }, timeoutMs = WEBCAM_ACCEPT_MS)

    suspend fun screenInput(kind: String, x: Float = 0f, y: Float = 0f, dx: Float = 0f, dy: Float = 0f, text: String? = null, key: String? = null, mods: List<String> = emptyList()) =
        ctx.send(packet(SCREEN_INPUT) {
            put("kind", kind); put("x", x); put("y", y); put("dx", dx); put("dy", dy)
            text?.let { put("text", it) }
            key?.let { put("key", it) }
            if (mods.isNotEmpty()) put("mods", kotlinx.serialization.json.JsonArray(mods.map { kotlinx.serialization.json.JsonPrimitive(it) }))
        })

    /** The PC's whole desktop. */
    val screenshot = Capture(SCREENSHOT_REQUEST, "PC screenshot")

    /** A photo from the PC's webcam (the PC shows a notification each time). */
    val photo = Capture(CAMERA_REQUEST, "PC camera")

    private val _rumble = MutableSharedFlow<Rumble>(extraBufferCapacity = 8)
    /** Rumble from the game on the PC, while the controller is open. */
    val rumble: SharedFlow<Rumble> = _rumble.asSharedFlow()
    private val _gamepadError = MutableStateFlow<String?>(null)
    /** Why the PC couldn't make the virtual pad (no access to /dev/uinput), once it says so. */
    val gamepadError: StateFlow<String?> = _gamepadError.asStateFlow()

    /** The PC's offer of a UDP port and key for the controller (see [streamGamepad]). */
    private class UdpOffer(val port: Int, val key: ByteArray)
    private val gamepadOffer = MutableStateFlow<UdpOffer?>(null)
    private val _gamepadRoute = MutableStateFlow<GamepadRoute?>(null)
    /** How the controller's states travel beside the link right now; null: only the link. */
    val gamepadRoute: StateFlow<GamepadRoute?> = _gamepadRoute.asStateFlow()
    private val _gamepadTrip = MutableStateFlow<TripReport?>(null)
    /** The controller's round trip on [gamepadRoute]; null until the first report there. */
    val gamepadTrip: StateFlow<TripReport?> = _gamepadTrip.asStateFlow()
    private val _pointerOpen = MutableStateFlow(false)
    /** Whether the touchpad's own Bluetooth channel is up. */
    val pointerOpen: StateFlow<Boolean> = _pointerOpen.asStateFlow()
    private val _pointerTrip = MutableStateFlow<TripReport?>(null)
    val pointerTrip: StateFlow<TripReport?> = _pointerTrip.asStateFlow()

    /**
     * The phone's USB cable without USB debugging (Android's accessory mode, set by the app): whether
     * it's plugged into a computer, the accessory's stream once the companion has switched it on, and
     * a way to drop that stream (failed, or the controller closed).
     */
    interface CableChannel {
        fun plugged(): Boolean
        fun open(): java.io.OutputStream?
        fun close()
        /** Android keeps listing an accessory for a moment after the cable is pulled: that one only fails. */
        fun openIfPlugged(): java.io.OutputStream? = if (plugged()) open() else null
    }
    @Volatile var cable: CableChannel? = null

    /** A radio channel for the controller's frames beside the link, steadier than Wi-Fi when there is one. */
    interface RadioChannel {
        /** Blocks while connecting; null when there's no way right now. Output first, then the echoes. */
        fun open(): Pair<java.io.OutputStream, java.io.InputStream>?
        fun close()
    }
    @Volatile var radio: RadioChannel? = null

    /** The PC this companion belongs to. */
    val pcId: String? get() = ctx.device.identity.featuresOf

    /**
     * Sends [states] to the PC's virtual pad for as long as this runs: every change at once (the flow
     * conflates, so a slow link only ever carries the newest), and the current one each second, which
     * keeps the pad alive (the PC lets go of everything after 4 s of silence). Ending it closes the pad.
     *
     * Once the PC offers its side channel, changes go there as signed frames instead: down the USB
     * cable when it's plugged in (the companion's adb reverse of [GAMEPAD_WIRED_PORT]; no Wi-Fi
     * stalls), else as UDP datagrams (when reached over a network), repeated every
     * [GAMEPAD_REPEAT_MS] so a lost one heals fast. A [radio] sits between them when there is one,
     * opened in the background since it takes seconds. The cable is tried again each second, so plugging
     * in mid-game moves over. The link still carries the once-a-second state. Every state is
     * numbered so the PC drops older ones.
     */
    suspend fun streamGamepad(states: StateFlow<GamepadState>) = withContext(Dispatchers.IO) {
        _gamepadError.value = null
        val sid = java.security.SecureRandom().nextInt()
        var seq = 0
        var used: UdpOffer? = null
        var socket: java.net.DatagramSocket? = null
        var wiredSocket: java.net.Socket? = null
        var wired: java.io.OutputStream? = null
        var cableWatch: CableWatch? = null
        var cableHeldUntil = 0L
        var lastAsk = 0L
        var mac: javax.crypto.Mac? = null
        var lastLink = 0L
        var radioOut: java.io.OutputStream? = null
        val radioReady = java.util.concurrent.atomic.AtomicReference<Pair<java.io.OutputStream, java.io.InputStream>?>(null)
        val radioUp = java.util.concurrent.atomic.AtomicBoolean(false)
        val trips = RoundTrips { _gamepadTrip.value = it }
        var lastReport = System.nanoTime() / 1_000_000
        fun route() {
            val now = when {
                wired != null -> GamepadRoute.USB
                radioOut != null -> GamepadRoute.BLUETOOTH
                socket != null -> GamepadRoute.WIFI
                else -> null
            }
            if (now != _gamepadRoute.value) {
                trips.report(_gamepadRoute.value)
                trips.moved()
                _gamepadTrip.value = null
            }
            _gamepadRoute.value = now
        }
        /** The PC echoes every so often, so the round trip is timed on the route frames really take. */
        fun listen(input: java.io.InputStream, watch: CableWatch? = null) {
            launch {
                runCatching {
                    val echoes = java.io.DataInputStream(input)
                    while (true) {
                        val seq = echoes.readInt()
                        trips.echoed(seq)
                        watch?.echoed()
                    }
                }
            }
        }
        fun listen(udp: java.net.DatagramSocket?) {
            if (udp != null) launch {
                val echo = java.net.DatagramPacket(ByteArray(4), 4)
                runCatching { while (true) { udp.receive(echo); trips.echoed(java.nio.ByteBuffer.wrap(echo.data).int) } }
            }
        }
        fun dropRadio() {
            runCatching { radioOut?.close() }
            radio?.close()
            radioOut = null
            radioUp.set(false)
        }
        val radioJob = launch {
            while (true) {
                // At once when the PC's offer lands: it's never there yet as the controller opens.
                gamepadOffer.first { it != null }
                if (!radioUp.get()) {
                    runCatching { radio?.open() }.getOrNull()?.let {
                        // Connecting takes a while: the controller may have closed meanwhile.
                        if (!isActive) {
                            runCatching { it.first.close() }
                            radio?.close()
                            return@launch
                        }
                        radioUp.set(true)
                        radioReady.set(it)
                    }
                }
                delay(GAMEPAD_RADIO_RETRY_MS)
            }
        }
        fun dropWired() {
            if (wiredSocket != null) runCatching { wiredSocket?.close() } else cable?.close()
            wiredSocket = null
            wired = null
            cableWatch = null
        }
        /**
         * Back to the other routes, and the cable left alone a moment: a pulled cable still accepts a
         * connection until adbd notices, and the phone still reports it plugged in for a second or two.
         */
        fun cableGone() {
            dropWired()
            cableHeldUntil = System.nanoTime() / 1_000_000 + GAMEPAD_CABLE_RETRY_MS
            socket = used?.let { ctx.device.datagramTo(it.port) }
            listen(socket)
            route()
            VLog.i(TAG, "controller cable gone; over ${_gamepadRoute.value ?: "the link"}")
        }
        /**
         * Moves to the cable if it's plugged in: adb's reverse (USB debugging on), else the USB
         * accessory; with neither, asks the companion to switch the phone into accessory mode.
         */
        suspend fun tryWired() {
            if (wired != null || used == null || System.nanoTime() / 1_000_000 < cableHeldUntil) return
            wiredSocket = runCatching {
                java.net.Socket().apply {
                    tcpNoDelay = true
                    connect(java.net.InetSocketAddress(java.net.InetAddress.getLoopbackAddress(), GAMEPAD_WIRED_PORT), 300)
                }
            }.getOrNull()
            wired = wiredSocket?.getOutputStream() ?: cable?.openIfPlugged()
            if (wired != null) {
                wiredSocket?.let {
                    cableWatch = CableWatch()
                    listen(it.getInputStream(), cableWatch)
                }
                socket?.close()
                socket = null
                VLog.i(TAG, "controller over the USB cable" + if (wiredSocket == null) " (accessory)" else "")
                route()
                if (wiredSocket != null) return
            }
            // No adb channel: the companion switches the phone to accessory mode, or (already switched)
            // starts reading it. Writes nobody reads fail after a moment and the ask comes again.
            val now = System.nanoTime() / 1_000_000
            if ((wired != null || cable?.plugged() == true) && now - lastAsk > ACCESSORY_ASK_MS) {
                lastAsk = now
                VLog.i(TAG, "asking the PC for USB accessory mode")
                ctx.send(packet(USB_ACCESSORY) { put("open", wired != null) })
            }
        }
        try {
            var current = states.value
            while (true) {
                val offer = gamepadOffer.value
                if (offer !== used) {
                    used = offer
                    socket?.close(); socket = null
                    dropWired()
                    mac = offer?.let { o -> javax.crypto.Mac.getInstance("HmacSHA256").apply { init(javax.crypto.spec.SecretKeySpec(o.key, "HmacSHA256")) } }
                    tryWired()
                    if (wired == null) {
                        socket = offer?.let { ctx.device.datagramTo(it.port) }
                        listen(socket)
                        if (socket != null) VLog.i(TAG, "controller over UDP to port ${offer?.port}")
                    }
                    route()
                }
                radioReady.getAndSet(null)?.let { (out, input) ->
                    radioOut = out
                    listen(input)
                    VLog.i(TAG, "controller over Bluetooth")
                    route()
                }
                seq++
                val now = System.nanoTime() / 1_000_000
                val frame = if (used != null) gamepadDatagram(current, sid, seq, mac!!) else null
                if (frame != null) trips.sending(seq)
                if (now - lastReport >= GAMEPAD_REPORT_MS) {
                    lastReport = now
                    trips.report(_gamepadRoute.value)
                }
                if (cableWatch?.silent(now) == true) cableGone()
                var fast = false
                wired?.let { w ->
                    fast = runCatching { w.write(frame!!); w.flush() }.isSuccess
                    // Unplugged (or the PC went).
                    if (fast) cableWatch?.sent(now) else cableGone()
                }
                if (!fast) radioOut?.let { r ->
                    fast = runCatching { r.write(frame!!); r.flush() }.isSuccess
                    if (!fast) {
                        dropRadio()
                        VLog.i(TAG, "controller's Bluetooth gone")
                        route()
                    }
                }
                if (!fast) socket?.let { s ->
                    fast = runCatching { s.send(java.net.DatagramPacket(frame!!, GAMEPAD_DATAGRAM)) }.isSuccess
                }
                if (!fast || now - lastLink >= GAMEPAD_HEARTBEAT_MS) {
                    tryWired()
                    lastLink = now
                    ctx.send(packet(GAMEPAD_STATE) {
                        put("b", current.buttons)
                        put("lx", current.lx); put("ly", current.ly)
                        put("rx", current.rx); put("ry", current.ry)
                        put("lt", current.lt); put("rt", current.rt)
                        put("kind", current.kind.wire)
                        put("mx", current.mx); put("my", current.my)
                        put("sid", sid.toLong() and 0xFFFFFFFFL); put("seq", seq.toLong() and 0xFFFFFFFFL)
                    })
                }
                val sent = current
                val wait = if (fast) GAMEPAD_REPEAT_MS else GAMEPAD_HEARTBEAT_MS
                current = withTimeoutOrNull(wait) { states.first { it != sent } } ?: sent
            }
        } finally {
            radioJob.cancel()
            radioReady.getAndSet(null)?.first?.let { runCatching { it.close() } }
            dropRadio()
            socket?.close()
            dropWired()
            trips.report(_gamepadRoute.value)
            _gamepadTrip.value = null
            gamepadOffer.value = null
            _gamepadRoute.value = null
            withContext(kotlinx.coroutines.NonCancellable) { ctx.send(packet(GAMEPAD_CLOSE)) }
        }
    }

    private fun receiveGamepadOffer(packet: NetworkPacket) {
        val port = packet.int("port") ?: return
        val key = packet.string("key")?.let { runCatching { java.util.Base64.getDecoder().decode(it) }.getOrNull() } ?: return
        if (port in 1..65535 && key.size >= 16) gamepadOffer.value = UdpOffer(port, key)
    }

    /** What the PC says about the phone's camera and microphone (see [sendWebcam]). */
    data class WebcamState(val camera: Boolean = false, val mic: Boolean = false, val error: String? = null)
    private val _webcam = MutableStateFlow(WebcamState())
    val webcam: StateFlow<WebcamState> = _webcam.asStateFlow()

    fun resetWebcam() { _webcam.value = WebcamState() }

    /**
     * The phone's camera as the PC's webcam: [video] is live H.264 (Annex-B), w x h as encoded,
     * [rotate] the clockwise turn that makes it upright. Suspends until the stream ends; false if the
     * PC never took it.
     */
    suspend fun sendWebcam(w: Int, h: Int, rotate: Int, video: java.io.InputStream): Boolean =
        ctx.device.sendWithPayload(
            packet(WEBCAM) { put("w", w); put("h", h); put("rotate", rotate) },
            PayloadSource(-1) { video }, timeoutMs = WEBCAM_ACCEPT_MS,
        )

    /** The phone's microphone as a PC input: [pcm] is live s16le audio. Suspends until it ends. */
    suspend fun sendMic(rate: Int, channels: Int, pcm: java.io.InputStream): Boolean =
        ctx.device.sendWithPayload(
            packet(WEBCAM_MIC) { put("rate", rate); put("channels", channels) },
            PayloadSource(-1) { pcm }, timeoutMs = WEBCAM_ACCEPT_MS,
        )

    /** The phone was turned (or switched cameras): the PC turns the picture without a restart. */
    suspend fun turnWebcam(rotate: Int) = ctx.send(packet(WEBCAM_TURN) { put("rotate", rotate) })

    private fun receiveWebcam(packet: NetworkPacket) {
        val error = packet.string("error")
        _webcam.value = _webcam.value.let { s ->
            if (error != null) s.copy(error = error)
            else s.copy(camera = packet.bool("camera") ?: s.camera, mic = packet.bool("mic") ?: s.mic, error = null)
        }
    }

    /**
     * One input from the PC's mirror window, in the picture's terms: [x], [y] are 0..1 across the
     * picture. Kinds: "down", "move", "up" (the left button, a finger), "scroll" ([dx], [dy] in
     * clicks, down positive), "text" ([text] typed), "key" ([key]: "backspace", "delete", "enter",
     * "left", "right", "up", "down"), "back", "home", "recents".
     */
    data class MirrorInput(
        val kind: String,
        val x: Float = 0f,
        val y: Float = 0f,
        val dx: Float = 0f,
        val dy: Float = 0f,
        val text: String? = null,
        val key: String? = null,
        /** With "key": Ctrl / Shift held (word steps, selecting). */
        val ctrl: Boolean = false,
        val shift: Boolean = false,
    )
    private val _mirrorInput = kotlinx.coroutines.flow.MutableSharedFlow<MirrorInput>(extraBufferCapacity = 512)
    val mirrorInput: kotlinx.coroutines.flow.SharedFlow<MirrorInput> = _mirrorInput

    /**
     * What the PC says about the mirror: [open] while its window shows the phone; [closed] once that
     * window was closed (the user stopping it); [error] why it can't.
     */
    /** [sound]: the codecs the PC's window can play the phone's sound in besides PCM ("opus"). */
    data class MirrorState(val open: Boolean = false, val closed: Boolean = false, val error: String? = null, val sound: List<String> = emptyList())
    private val _mirror = MutableStateFlow(MirrorState())
    val mirror: StateFlow<MirrorState> = _mirror.asStateFlow()

    fun resetMirror() { _mirror.value = MirrorState() }

    /**
     * The phone's screen in a window on the PC: [video] is live H.264 (Annex-B), w x h to start with
     * (a turned phone sends a new size in-stream). Suspends until the stream ends.
     */
    /** A mirror's starting: the last one's state (its window, its codecs) no longer holds. */
    fun mirrorStarting() {
        _mirror.value = MirrorState()
    }

    suspend fun sendMirror(w: Int, h: Int, video: java.io.InputStream): Boolean =
        ctx.device.sendWithPayload(
            packet(MIRROR) { put("w", w); put("h", h) },
            PayloadSource(-1) { video }, timeoutMs = WEBCAM_ACCEPT_MS,
        )

    /**
     * The phone's sound while mirrored: [sound] is live s16le audio, or with [codec] "opus" Opus
     * packets each after a 2-byte big-endian length. Suspends until it ends.
     */
    suspend fun sendMirrorAudio(rate: Int, channels: Int, sound: java.io.InputStream, codec: String? = null): Boolean =
        ctx.device.sendWithPayload(
            packet(MIRROR_AUDIO) { put("rate", rate); put("channels", channels); codec?.let { put("codec", it) } },
            PayloadSource(-1) { sound }, timeoutMs = WEBCAM_ACCEPT_MS,
        )

    /** Text copied in the mirror (Ctrl+C / Ctrl+X in the PC's window), for the PC's clipboard. */
    suspend fun mirrorCopy(text: String) = ctx.send(packet(MIRROR_COPY) { put("text", text) })

    /** A volume key while the phone's sound plays on the PC: the PC's volume, a step. */
    suspend fun mirrorVolume(up: Boolean) = ctx.send(packet(MIRROR_VOLUME) { put("up", up) })

    private fun receiveMirrorInput(p: NetworkPacket) {
        val kind = p.string("kind") ?: return
        fun f(k: String) = p.double(k)?.toFloat() ?: 0f
        _mirrorInput.tryEmit(
            MirrorInput(kind, f("x"), f("y"), f("dx"), f("dy"), p.string("text"), p.string("key"), p.bool("ctrl") ?: false, p.bool("shift") ?: false),
        )
    }

    private fun receiveMirrorState(p: NetworkPacket) {
        val open = p.bool("open") ?: false
        _mirror.value = MirrorState(open = open, closed = !open && _mirror.value.open, error = p.string("error"), sound = p.stringList("sound"))
    }

    @Volatile private var pendingStep: CompletableDeferred<NetworkPacket>? = null

    /**
     * One three-finger step on a Plasma desktop (up: desktop → windows → Overview → Grid, down back),
     * decided by the companion from where KWin actually is. Returns where it ended up
     * ("desktop", "windows", "overview", "grid").
     */
    suspend fun desktopStep(up: Boolean): Result<String> {
        val reply = CompletableDeferred<NetworkPacket>()
        pendingStep = reply
        if (!ctx.send(packet(DESKTOP_STEP) { put("up", up) })) return Result.failure(IllegalStateException("The PC isn't connected"))
        val p = withTimeoutOrNull(STEP_TIMEOUT_MS) { reply.await() } ?: return Result.failure(IllegalStateException("The PC didn't answer"))
        p.string("error")?.let { return Result.failure(IllegalStateException(it)) }
        return Result.success(p.string("view") ?: "windows")
    }

    private var pendingPointer: CompletableDeferred<NetworkPacket>? = null

    /**
     * The touchpad's own Bluetooth channel: the radio first, so the PC is never asked to allow control
     * for a channel that can't connect, then a key over the link. Success(null): no radio right now, worth
     * trying again; failure: the PC said no (or has no such thing), so don't keep asking.
     */
    suspend fun openPointer(radio: RadioChannel): Result<PointerChannel?> = withContext(kotlinx.coroutines.Dispatchers.IO) {
        val (out, input) = radio.open() ?: return@withContext Result.success(null)
        var handedOver = false
        try {
        val reply = CompletableDeferred<NetworkPacket>()
        pendingPointer = reply
        val answer = if (ctx.send(packet(POINTER_REQUEST) {})) withTimeoutOrNull(POINTER_TIMEOUT_MS) { reply.await() } else null
        val key = answer?.string("key")?.let { runCatching { java.util.Base64.getDecoder().decode(it) }.getOrNull() }
        if (key == null) {
            return@withContext Result.failure(IllegalStateException(answer?.string("error") ?: "The PC didn't answer"))
        }
        _pointerOpen.value = true
        handedOver = true
        Result.success(PointerChannel(out, input, key, { _pointerTrip.value = it }) {
            _pointerOpen.value = false
            _pointerTrip.value = null
            radio.close()
        })
        } finally {
            if (!handedOver) radio.close() // refused, or the page was left while the PC's dialog was up
        }
    }

    fun closePointer() {
        ctx.scope.launch { ctx.send(packet(POINTER_CLOSE) {}) }
    }

    /** One UDP state, laid out as the companion's gamepad.rs reads it. */
    private fun gamepadDatagram(s: GamepadState, sid: Int, seq: Int, mac: javax.crypto.Mac): ByteArray {
        val b = java.nio.ByteBuffer.allocate(GAMEPAD_DATAGRAM)
        b.put("VYG1".toByteArray()).putInt(sid).putInt(seq).putInt(s.buttons)
        for (v in intArrayOf(s.lx, s.ly, s.rx, s.ry)) b.putShort(v.coerceIn(-32768, 32767).toShort())
        b.put(s.lt.coerceIn(0, 255).toByte()).put(s.rt.coerceIn(0, 255).toByte())
        b.putInt(s.mx).putInt(s.my).put(if (s.kind == PadKind.DUALSENSE) 1 else 0)
        val body = b.array().copyOf(GAMEPAD_BODY)
        b.put(mac.doFinal(body), 0, GAMEPAD_DATAGRAM - GAMEPAD_BODY)
        return b.array()
    }

    private fun receiveRumble(packet: NetworkPacket) {
        packet.string("error")?.let { _gamepadError.value = it; return }
        _rumble.tryEmit(Rumble(packet.int("strong") ?: 0, packet.int("weak") ?: 0, packet.int("ms") ?: 0))
    }

    private val _refused = MutableSharedFlow<String>(extraBufferCapacity = 4, onBufferOverflow = kotlinx.coroutines.channels.BufferOverflow.DROP_OLDEST)
    /** Something was refused (or stopped) because it's turned off for this phone on the PC, and nothing waiting shows it: say so. */
    val refused: SharedFlow<String> = _refused.asSharedFlow()

    /**
     * The PC turned [DENIED]'s feature off for this phone: whatever was waiting on [request] gets
     * the reason as its error; what has no waiter (a wake, a stopped stream) goes to [refused].
     */
    private fun receiveDenied(packet: NetworkPacket) {
        val name = packet.string("name") ?: "That"
        val why = "Turned off for this phone on the PC. Allow \u201c$name\u201d in Vyoma there."
        val request = packet.string("request").orEmpty()
        val path = packet.string("path")
        val key = packet.string("key")
        fun error(type: String) = NetworkPacket(type, buildJsonObject {
            put("error", why)
            path?.let { put("path", it) }
            key?.let { put("key", it) }
        })
        when (request) {
            SCREENSHOT_REQUEST -> screenshot.receive(error(SCREENSHOT))
            CAMERA_REQUEST -> photo.receive(error(CAMERA))
            SCREEN_REQUEST -> pendingScreen?.also { pendingScreen = null }?.complete(error(SCREEN)) ?: _refused.tryEmit(why)
            POINTER_REQUEST -> pendingPointer?.also { pendingPointer = null }?.complete(error(POINTER_READY)) ?: _refused.tryEmit(why)
            DESKTOP_STEP -> pendingStep?.also { pendingStep = null }?.complete(error(DESKTOP_STEPPED)) ?: _refused.tryEmit(why)
            GAMEPAD_STATE -> _gamepadError.value = why
            WEBCAM, WEBCAM_MIC -> receiveWebcam(error(WEBCAM_STATE))
            PASTE -> pendingPaste?.also { pendingPaste = null }?.complete(why) ?: _refused.tryEmit(why)
            PC_POWER -> pendingPower?.also { pendingPower = null }?.complete(why) ?: _refused.tryEmit(why)
            FILES_LIST -> _folder.update { if (it?.loading == true) it.copy(loading = false, error = why) else it }
            FILES_GET -> if (path != null) receiveFile(error(FILES_FILE)) else _refused.tryEmit(why)
            FILES_PUT -> pending?.also { it.reply.complete(it.name to why) } ?: _refused.tryEmit(why)
            XFER_BEGIN -> xferReplies.remove("ready:$key")?.complete(error(XFER_READY))
            XFER_DATA -> xferReplies.remove("done:$key")?.complete(error(XFER_DONE))
            else -> _refused.tryEmit(why)
        }
    }

    override suspend fun onPacket(packet: NetworkPacket): Boolean {
        if (packet.type == DENIED) receiveDenied(packet)
        if (packet.type == GAMEPAD_RUMBLE) receiveRumble(packet)
        if (packet.type == WEBCAM_STATE) receiveWebcam(packet)
        if (packet.type == MIRROR_INPUT) receiveMirrorInput(packet)
        if (packet.type == MIRROR_STATE) receiveMirrorState(packet)
        if (packet.type == GAMEPAD_UDP) receiveGamepadOffer(packet)
        if (packet.type == POINTER_READY) pendingPointer?.also { pendingPointer = null }?.complete(packet)
        if (packet.type == DESKTOP_STEPPED) pendingStep?.also { pendingStep = null }?.complete(packet)
        if (packet.type == SCREENSHOT) screenshot.receive(packet)
        if (packet.type == CAMERA) photo.receive(packet)
        if (packet.type == HANDOFF) receiveHandoff(packet)
        if (packet.type == SCREEN) pendingScreen?.also { pendingScreen = null }?.complete(packet)
        if (packet.type == FILES_LISTING) receiveListing(packet)
        if (packet.type == FILES_FILE) receiveFile(packet)
        if (packet.type == FILES_SAVED) receiveSaved(packet)
        if (packet.type == PC) receiveControls(packet)
        if (packet.type == WALLPAPER) receiveWallpaper(packet)
        if (packet.type == SPECS) PcSpecs.parse(packet.body)?.let { _specs.value = it to packet.body }
        if (packet.type == BLUETOOTH) packet.string("address")?.let { _bluetoothAddress.value = it }
        if (packet.type == XFER_READY) receiveXfer("ready", packet)
        if (packet.type == XFER_DONE) receiveXfer("done", packet)
        if (packet.type == PASTE_READY) {
            val waiting = pendingPaste
            if (waiting != null) { pendingPaste = null; waiting.complete(packet.string("error")) }
            else if (packet.string("error") != null) _typingFailed.tryEmit(Unit)
        }
        if (packet.type == PC_POWERED) pendingPower?.also { pendingPower = null }?.complete(packet.string("error"))
        if (packet.type == STATS) {
            _stats.value = PcStats(
                cpu = packet.double("cpu")?.toFloat() ?: 0f,
                memUsed = packet.long("memUsed") ?: 0,
                memTotal = packet.long("memTotal") ?: 0,
                cpuTemp = packet.double("cpuTemp")?.toFloat(),
                gpu = packet.int("gpu"),
                uptimeSeconds = packet.long("uptime"),
                handoff = packet.string("handoff"),
                camera = packet.bool("camera"),
                cameraLid = packet.bool("cameraLid"),
                ac = packet.bool("ac"),
            )
        }
        return true
    }

    companion object {
        const val ID = "vyoma_companion"
        const val STATS = "vyoma.stats"
        const val STATS_REQUEST = "vyoma.stats.request"
        const val SCREENSHOT = "vyoma.screenshot"
        const val SCREENSHOT_REQUEST = "vyoma.screenshot.request"
        const val CAMERA = "vyoma.camera"
        const val CAMERA_REQUEST = "vyoma.camera.request"
        const val HANDOFF = "vyoma.handoff"
        const val HANDOFF_REQUEST = "vyoma.handoff.request"
        private const val HANDOFF_TIMEOUT_MS = 5_000L
        const val FILES_LIST = "vyoma.files.list"
        const val FILES_LISTING = "vyoma.files.listing"
        const val FILES_GET = "vyoma.files.get"
        const val FILES_FILE = "vyoma.files.file"
        const val FILES_PUT = "vyoma.files.put"
        const val FILES_SAVED = "vyoma.files.saved"
        const val SCREEN = "vyoma.screen"
        const val SCREEN_REQUEST = "vyoma.screen.request"
        const val SCREEN_STOP = "vyoma.screen.stop"
        const val SCREEN_INPUT = "vyoma.screen.input"
        const val SCREEN_TOUCH = "vyoma.screen.touch"
        private const val SCREEN_TIMEOUT_MS = 60_000L
        const val PC = "vyoma.pc"
        const val WALLPAPER = "vyoma.wallpaper"
        const val SPECS = "vyoma.specs"
        const val SPECS_REQUEST = "vyoma.specs.request"
        const val BLUETOOTH = "vyoma.bluetooth"
        private const val GAMEPAD_RADIO_RETRY_MS = 10_000L
        private const val GAMEPAD_REPORT_MS = 30_000L
        const val PASTE = "vyoma.paste"
        const val POINTER_REQUEST = "vyoma.pointer.request"
        const val POINTER_READY = "vyoma.pointer.ready"
        const val POINTER_CLOSE = "vyoma.pointer.close"
        /** Long enough for someone to answer the PC's "allow control" dialog. */
        const val POINTER_TIMEOUT_MS = 60_000L
        const val PASTE_READY = "vyoma.paste.ready"
        private const val PASTE_WAIT_MS = 3_000L
        const val WALLPAPER_REQUEST = "vyoma.wallpaper.request"
        const val PC_REQUEST = "vyoma.pc.request"
        const val PC_SET = "vyoma.pc.set"
        const val PC_POWER = "vyoma.pc.power"
        const val PC_POWERED = "vyoma.pc.powered"
        const val PC_WAKE = "vyoma.pc.wake"
        const val AWAY = "vyoma.away"
        const val GAMEPAD_STATE = "vyoma.gamepad.state"
        const val GAMEPAD_CLOSE = "vyoma.gamepad.close"
        const val GAMEPAD_RUMBLE = "vyoma.gamepad.rumble"
        const val GAMEPAD_UDP = "vyoma.gamepad.udp"
        private const val GAMEPAD_HEARTBEAT_MS = 1_000L
        private const val GAMEPAD_REPEAT_MS = 100L
        private const val GAMEPAD_CABLE_RETRY_MS = 3_000L
        /** The phone's end of the companion's adb reverse for the wired controller; must match usb.rs WIRED. */
        const val GAMEPAD_WIRED_PORT = 1866
        const val USB_ACCESSORY = "vyoma.usb.accessory"
        const val WEBCAM = "vyoma.webcam"
        const val WEBCAM_MIC = "vyoma.webcam.mic"
        const val WEBCAM_TURN = "vyoma.webcam.turn"
        const val WEBCAM_STATE = "vyoma.webcam.state"
        const val MIRROR = "vyoma.mirror"
        const val MIRROR_INPUT = "vyoma.mirror.input"
        const val MIRROR_AUDIO = "vyoma.mirror.audio"
        const val MIRROR_COPY = "vyoma.mirror.copy"
        const val MIRROR_VOLUME = "vyoma.mirror.volume"
        const val MIRROR_STATE = "vyoma.mirror.state"
        /** A request refused: its feature is turned off for this phone on the PC. */
        const val DENIED = "vyoma.denied"
        private const val WEBCAM_ACCEPT_MS = 15_000
        private const val ACCESSORY_ASK_MS = 15_000L
        private const val GAMEPAD_BODY = 35
        private const val GAMEPAD_DATAGRAM = 51
        const val DESKTOP_STEP = "vyoma.desktop.step"
        const val DESKTOP_STEPPED = "vyoma.desktop.stepped"
        private const val STEP_TIMEOUT_MS = 3_000L
        /** The phone's walk-away rules, set by the app; every companion gets them on link and on change. */
        val away = kotlinx.coroutines.flow.MutableStateFlow(AwayRules())
        private const val SET_INTERVAL_MS = 60L
        const val XFER_BEGIN = "vyoma.xfer.begin"
        const val XFER_READY = "vyoma.xfer.ready"
        const val XFER_DATA = "vyoma.xfer.data"
        const val XFER_DONE = "vyoma.xfer.done"
        const val XFER_END = "vyoma.xfer.end"
        private const val XFER_REPLY_MS = 10_000L
        /** Marks the stand-in answer [onStop] gives sends that were waiting. */
        private const val LOST = "vyomaLost"
        /** After the last byte: the PC syncs it to disk and renames it before answering. */
        private const val XFER_SAVE_MS = 60_000L
        private const val POWER_TIMEOUT_MS = 5_000L
        private const val CAPTURE_TIMEOUT_MS = 30_000L
        private const val TAG = "Companion"
        private val IMAGE_FORMATS = setOf("png", "webp", "jpg")
        fun spec(sink: ShareSink) = PluginSpec(
            ID,
            incoming = setOf(POINTER_READY, BLUETOOTH, PASTE_READY, STATS, SCREENSHOT, CAMERA, HANDOFF, FILES_LISTING, FILES_FILE, FILES_SAVED, SCREEN, PC, PC_POWERED, XFER_READY, XFER_DONE, WALLPAPER, SPECS, GAMEPAD_RUMBLE, GAMEPAD_UDP, DESKTOP_STEPPED, WEBCAM_STATE, MIRROR_INPUT, MIRROR_STATE, DENIED),
            outgoing = setOf(POINTER_REQUEST, POINTER_CLOSE, PASTE, STATS_REQUEST, SCREENSHOT_REQUEST, CAMERA_REQUEST, HANDOFF_REQUEST, FILES_LIST, FILES_GET, FILES_PUT, SCREEN_REQUEST, SCREEN_STOP, SCREEN_INPUT, SCREEN_TOUCH, PC_REQUEST, PC_SET, PC_POWER, PC_WAKE, AWAY, XFER_BEGIN, XFER_DATA, XFER_END, WALLPAPER_REQUEST, SPECS_REQUEST, GAMEPAD_STATE, GAMEPAD_CLOSE, DESKTOP_STEP, USB_ACCESSORY, WEBCAM, WEBCAM_MIC, WEBCAM_TURN, MIRROR, MIRROR_AUDIO, MIRROR_COPY, MIRROR_VOLUME),
        ) { CompanionPlugin(it, sink) }
    }
}

/** Skips exactly [n] bytes (a plain skip may stop short), or fails if the stream ends first. */
internal fun java.io.InputStream.skipFully(n: Long) {
    var left = n
    while (left > 0) {
        val skipped = skip(left)
        if (skipped > 0) left -= skipped else if (read() >= 0) left-- else throw java.io.EOFException("ended $left bytes early")
    }
}
