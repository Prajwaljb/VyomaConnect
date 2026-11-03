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
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
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
    /** On its charger, whether or not the battery is taking charge (a charge limit holds it); null: not said. */
    val ac: Boolean? = null,
)

/**
 * The phone's game controller as the PC's virtual Xbox 360 pad sees it: [buttons] a bitmask of
 * [GamepadButton]s, sticks -32768..32767 (y down), triggers 0..255.
 */
data class GamepadState(
    val buttons: Int = 0,
    val lx: Int = 0,
    val ly: Int = 0,
    val rx: Int = 0,
    val ry: Int = 0,
    val lt: Int = 0,
    val rt: Int = 0,
)

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
data class AwayRules(val lock: Boolean = false, val pause: Boolean = false, val mute: Boolean = false, val afterSeconds: Int = 0, val undo: Boolean = false)

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

    private val _wallpaper = MutableStateFlow<PcWallpaper?>(null)
    /** The PC's wallpaper colours, asked for on every link (it may have changed since). */
    val wallpaper: StateFlow<PcWallpaper?> = _wallpaper.asStateFlow()

    suspend fun requestWallpaper() = ctx.send(packet(WALLPAPER_REQUEST))

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
                    val next = synchronized(pendingSets) { pendingSets.values.toList().also { pendingSets.clear() } }
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
    }

    override suspend fun onStart() {
        sendAway()
        requestWallpaper()
    }

    /** Tells the PC the current walk-away rules ([away]); sent on every link too, so a restarted companion knows them. */
    suspend fun sendAway() {
        val rules = away.value
        ctx.send(packet(AWAY) {
            put("lock", rules.lock); put("pause", rules.pause); put("mute", rules.mute)
            put("after", rules.afterSeconds); put("undo", rules.undo)
        })
    }

    private fun abandonReplies() {
        for (key in xferReplies.keys.toList()) xferReplies.remove(key)?.complete(packet(XFER_DONE) { put(LOST, true) })
    }

    private fun receiveXfer(prefix: String, packet: NetworkPacket) {
        val key = packet.string("key") ?: return
        xferReplies.remove("$prefix:$key")?.complete(packet)
    }

    /** Turns the PC's screens back on and unlocks it (the stock unlock left a dark screen dark). */
    suspend fun wake() = ctx.send(packet(PC_WAKE))

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
    class ScreenStream(val width: Int, val height: Int, val video: java.io.InputStream)

    private var pendingScreen: CompletableDeferred<NetworkPacket>? = null

    /**
     * Starts a live cast of the PC's screen. The first time, Plasma asks on the PC which screen to
     * share, so this waits up to a minute for the answer.
     */
    suspend fun openScreen(): Result<ScreenStream> {
        val reply = CompletableDeferred<NetworkPacket>()
        pendingScreen = reply
        if (!ctx.send(packet(SCREEN_REQUEST))) return Result.failure(IllegalStateException("The PC isn't reachable"))
        val answer = withTimeoutOrNull(SCREEN_TIMEOUT_MS) { reply.await() }
            ?: return Result.failure(IllegalStateException("The PC didn't answer. Is screen sharing waiting for you there?"))
        answer.string("error")?.let { return Result.failure(IllegalStateException(it)) }
        if (!answer.hasPayload) return Result.failure(IllegalStateException("The PC sent no video"))
        return runCatching {
            val video = withContext(Dispatchers.IO) { ctx.device.openPayload(answer) } ?: error("Couldn't open the video stream")
            ScreenStream(answer.int("width") ?: 0, answer.int("height") ?: 0, video)
        }
    }

    /** Ends the cast on the PC (closing the stream does too; this is quicker and says so). */
    suspend fun stopScreen() = ctx.send(packet(SCREEN_STOP))

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

    /**
     * Sends [states] to the PC's virtual pad for as long as this runs: every change at once (the flow
     * conflates, so a slow link only ever carries the newest), and the current one each second, which
     * keeps the pad alive (the PC lets go of everything after 4 s of silence). Ending it closes the pad.
     */
    suspend fun streamGamepad(states: StateFlow<GamepadState>) {
        _gamepadError.value = null
        try {
            var current = states.value
            while (true) {
                ctx.send(packet(GAMEPAD_STATE) {
                    put("b", current.buttons)
                    put("lx", current.lx); put("ly", current.ly)
                    put("rx", current.rx); put("ry", current.ry)
                    put("lt", current.lt); put("rt", current.rt)
                })
                val sent = current
                current = withTimeoutOrNull(GAMEPAD_HEARTBEAT_MS) { states.first { it != sent } } ?: sent
            }
        } finally {
            withContext(kotlinx.coroutines.NonCancellable) { ctx.send(packet(GAMEPAD_CLOSE)) }
        }
    }

    private fun receiveRumble(packet: NetworkPacket) {
        packet.string("error")?.let { _gamepadError.value = it; return }
        _rumble.tryEmit(Rumble(packet.int("strong") ?: 0, packet.int("weak") ?: 0, packet.int("ms") ?: 0))
    }

    override suspend fun onPacket(packet: NetworkPacket): Boolean {
        if (packet.type == GAMEPAD_RUMBLE) receiveRumble(packet)
        if (packet.type == SCREENSHOT) screenshot.receive(packet)
        if (packet.type == CAMERA) photo.receive(packet)
        if (packet.type == HANDOFF) receiveHandoff(packet)
        if (packet.type == SCREEN) pendingScreen?.also { pendingScreen = null }?.complete(packet)
        if (packet.type == FILES_LISTING) receiveListing(packet)
        if (packet.type == FILES_FILE) receiveFile(packet)
        if (packet.type == FILES_SAVED) receiveSaved(packet)
        if (packet.type == PC) receiveControls(packet)
        if (packet.type == WALLPAPER) receiveWallpaper(packet)
        if (packet.type == XFER_READY) receiveXfer("ready", packet)
        if (packet.type == XFER_DONE) receiveXfer("done", packet)
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
        private const val SCREEN_TIMEOUT_MS = 60_000L
        const val PC = "vyoma.pc"
        const val WALLPAPER = "vyoma.wallpaper"
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
        private const val GAMEPAD_HEARTBEAT_MS = 1_000L
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
            incoming = setOf(STATS, SCREENSHOT, CAMERA, HANDOFF, FILES_LISTING, FILES_FILE, FILES_SAVED, SCREEN, PC, PC_POWERED, XFER_READY, XFER_DONE, WALLPAPER, GAMEPAD_RUMBLE),
            outgoing = setOf(STATS_REQUEST, SCREENSHOT_REQUEST, CAMERA_REQUEST, HANDOFF_REQUEST, FILES_LIST, FILES_GET, FILES_PUT, SCREEN_REQUEST, SCREEN_STOP, PC_REQUEST, PC_SET, PC_POWER, PC_WAKE, AWAY, XFER_BEGIN, XFER_DATA, XFER_END, WALLPAPER_REQUEST, GAMEPAD_STATE, GAMEPAD_CLOSE),
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
