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
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonPrimitive
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
)

/** Where a screenshot request is. [Done.location] is something the UI can open. */
sealed interface ScreenshotState {
    data object Idle : ScreenshotState
    data object Capturing : ScreenshotState
    data class Done(val location: String, val mime: String, val at: Long) : ScreenshotState
    data class Failed(val reason: String, val at: Long) : ScreenshotState
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

sealed interface PcFileEvent {
    data class Saved(val name: String, val location: String) : PcFileEvent
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

    private val _screenshot = MutableStateFlow<ScreenshotState>(ScreenshotState.Idle)
    val screenshot: StateFlow<ScreenshotState> = _screenshot.asStateFlow()
    private var watchdog: Job? = null

    private val _folder = MutableStateFlow<PcFolder?>(null)
    val folder: StateFlow<PcFolder?> = _folder.asStateFlow()

    /** PC paths being downloaded right now. */
    private val _fetching = MutableStateFlow<Set<String>>(emptySet())
    val fetching: StateFlow<Set<String>> = _fetching.asStateFlow()
    private val _fileEvents = MutableSharedFlow<PcFileEvent>(extraBufferCapacity = 8)
    val fileEvents: SharedFlow<PcFileEvent> = _fileEvents.asSharedFlow()

    suspend fun requestStats() = ctx.send(packet(STATS_REQUEST))

    /** Lists a home-relative PC folder; [folder] shows it loading, then the result. */
    suspend fun listFiles(path: String) {
        val previous = _folder.value
        // Keep the old entries on a refresh of the same folder so the list doesn't flash empty.
        _folder.value = if (previous?.path == path) previous.copy(loading = true, error = null) else PcFolder(path)
        if (!ctx.send(packet(FILES_LIST) { put("path", path) })) {
            _folder.update { if (it?.path == path) it.copy(loading = false, error = "The PC isn't reachable") else it }
        }
    }

    /** Downloads a PC file into the phone's Downloads; progress shows like any received file. */
    suspend fun download(path: String) {
        if (path in _fetching.value) return
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
        _folder.update { current ->
            if (current != null && current.path != path) return@update current // the user has moved on
            PcFolder(path, entries, loading = false, error = packet.string("error"), truncated = packet.bool("truncated") == true)
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
        ctx.scope.launch(Dispatchers.IO) {
            val size = packet.payloadSize
            val id = Transfers.start(ctx.device, name, incoming = true, total = size)
            val target = sink.createFile(ctx.device, name, size, null)
            if (target == null) {
                Transfers.finish(id, ok = false)
                return@launch done(PcFileEvent.Failed(name, "Couldn't save it"))
            }
            try {
                val input = ctx.device.openPayload(packet) ?: error("no payload")
                input.use { it.copyWithProgress(target.output) { n -> Transfers.progress(id, n) } }
                target.output.close()
                val location = target.complete() ?: error("not saved")
                Transfers.finish(id, ok = true, location = location)
                done(PcFileEvent.Saved(name, location))
            } catch (e: Exception) {
                VLog.w(TAG, "file transfer failed: ${e.message}")
                target.abort()
                Transfers.finish(id, ok = false)
                done(PcFileEvent.Failed(name, "The transfer was interrupted"))
            }
        }
    }

    /** Asks the PC for a screenshot of its whole desktop; [screenshot] follows it to Done or Failed. */
    suspend fun requestScreenshot() {
        if (_screenshot.value == ScreenshotState.Capturing) return
        _screenshot.value = ScreenshotState.Capturing
        if (!ctx.send(packet(SCREENSHOT_REQUEST))) return fail("The PC isn't reachable")
        watchdog?.cancel()
        watchdog = ctx.scope.launch {
            delay(SCREENSHOT_TIMEOUT_MS)
            if (_screenshot.value == ScreenshotState.Capturing) fail("The PC didn't answer")
        }
    }

    private fun fail(reason: String) {
        _screenshot.value = ScreenshotState.Failed(reason, System.currentTimeMillis())
    }

    private fun receiveScreenshot(packet: NetworkPacket) {
        packet.string("error")?.let { return fail(it) }
        if (!packet.hasPayload) return fail("The PC sent no image")
        ctx.scope.launch(Dispatchers.IO) {
            val ext = packet.string("format")?.takeIf { it in IMAGE_FORMATS } ?: "png"
            val name = "PC screenshot " + SimpleDateFormat("yyyy-MM-dd HH.mm.ss", Locale.US).format(Date()) + ".$ext"
            val target = sink.createPicture(ctx.device, name, packet.payloadSize) ?: return@launch fail("Couldn't save the image")
            try {
                val input = ctx.device.openPayload(packet) ?: error("no payload")
                input.use { it.copyTo(target.output, 256 * 1024) }
                target.output.close()
                val location = target.complete() ?: error("not saved")
                watchdog?.cancel()
                _screenshot.value = ScreenshotState.Done(location, if (ext == "jpg") "image/jpeg" else "image/$ext", System.currentTimeMillis())
            } catch (e: Exception) {
                VLog.w(TAG, "screenshot transfer failed: ${e.message}")
                target.abort()
                fail("The transfer was interrupted")
            }
        }
    }

    override suspend fun onPacket(packet: NetworkPacket): Boolean {
        if (packet.type == SCREENSHOT) receiveScreenshot(packet)
        if (packet.type == FILES_LISTING) receiveListing(packet)
        if (packet.type == FILES_FILE) receiveFile(packet)
        if (packet.type == FILES_SAVED) receiveSaved(packet)
        if (packet.type == STATS) {
            _stats.value = PcStats(
                cpu = packet.double("cpu")?.toFloat() ?: 0f,
                memUsed = packet.long("memUsed") ?: 0,
                memTotal = packet.long("memTotal") ?: 0,
                cpuTemp = packet.double("cpuTemp")?.toFloat(),
                gpu = packet.int("gpu"),
                uptimeSeconds = packet.long("uptime"),
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
        const val FILES_LIST = "vyoma.files.list"
        const val FILES_LISTING = "vyoma.files.listing"
        const val FILES_GET = "vyoma.files.get"
        const val FILES_FILE = "vyoma.files.file"
        const val FILES_PUT = "vyoma.files.put"
        const val FILES_SAVED = "vyoma.files.saved"
        private const val SCREENSHOT_TIMEOUT_MS = 30_000L
        private const val TAG = "Companion"
        private val IMAGE_FORMATS = setOf("png", "webp", "jpg")
        fun spec(sink: ShareSink) = PluginSpec(
            ID,
            incoming = setOf(STATS, SCREENSHOT, FILES_LISTING, FILES_FILE, FILES_SAVED),
            outgoing = setOf(STATS_REQUEST, SCREENSHOT_REQUEST, FILES_LIST, FILES_GET, FILES_PUT),
        ) { CompanionPlugin(it, sink) }
    }
}
