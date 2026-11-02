package dev.vyoma.connect.protocol.plugin.builtin

import dev.vyoma.connect.protocol.NetworkPacket
import dev.vyoma.connect.protocol.PacketTypes
import dev.vyoma.connect.protocol.VLog
import dev.vyoma.connect.protocol.device.Device
import dev.vyoma.connect.protocol.link.PayloadSource
import dev.vyoma.connect.protocol.link.copyWithProgress
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
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.put
import java.io.InputStream
import java.io.OutputStream
import java.util.concurrent.atomic.AtomicLong

/** A file transfer in either direction, for progress UI (notifications, Live Updates, widgets). */
data class Transfer(
    val id: Long,
    val deviceId: String,
    val deviceName: String,
    val fileName: String,
    val incoming: Boolean,
    val totalBytes: Long,
    val doneBytes: Long = 0,
    val state: State = State.RUNNING,
    /** Where an incoming file landed (platform URI), for "open" actions. */
    val location: String? = null,
) {
    enum class State { RUNNING, DONE, FAILED }

    val fraction: Float get() = if (totalBytes > 0) (doneBytes.toFloat() / totalBytes).coerceIn(0f, 1f) else 0f
}

/** Every transfer, across devices. Finished ones stay until [dismiss] so the UI can show the result. */
object Transfers {
    private val ids = AtomicLong(1)
    private val _all = MutableStateFlow<List<Transfer>>(emptyList())
    val all: StateFlow<List<Transfer>> = _all.asStateFlow()

    internal fun start(device: Device, name: String, incoming: Boolean, total: Long): Long {
        val id = ids.getAndIncrement()
        _all.update { it + Transfer(id, device.id, device.name, name, incoming, total) }
        return id
    }

    internal fun progress(id: Long, done: Long) = edit(id) { it.copy(doneBytes = done) }

    internal fun finish(id: Long, ok: Boolean, location: String? = null) = edit(id) {
        it.copy(state = if (ok) Transfer.State.DONE else Transfer.State.FAILED, doneBytes = if (ok) it.totalBytes else it.doneBytes, location = location)
    }

    fun dismiss(id: Long) = _all.update { list -> list.filterNot { it.id == id } }
    fun dismissFinished() = _all.update { list -> list.filter { it.state == Transfer.State.RUNNING } }

    private inline fun edit(id: Long, crossinline f: (Transfer) -> Transfer) =
        _all.update { list -> list.map { if (it.id == id) f(it) else it } }
}

/** A file we're about to send. [open] is called once the PC connects. */
class OutgoingFile(val name: String, val size: Long, val lastModified: Long?, val open: () -> InputStream)

/** Where incoming shares go on this platform. */
interface ShareSink {
    fun onText(from: Device, text: String)
    fun onUrl(from: Device, url: String)

    /** Create the destination for an incoming file; null refuses it. */
    fun createFile(from: Device, name: String, size: Long, lastModified: Long?): IncomingFile?

    /** Like [createFile], for images that belong with the user's pictures (PC screenshots). */
    fun createPicture(from: Device, name: String, size: Long): IncomingFile? = createFile(from, name, size, null)

    /** A file only to look at: kept out of the user's files and cleared on its own. Platforms without such a place save it. */
    fun createTemporary(from: Device, name: String, size: Long): IncomingFile? = createFile(from, name, size, null)
}

interface IncomingFile {
    val output: OutputStream
    /** Publish the finished file; returns a location the UI can open. */
    fun complete(): String?
    fun abort()
}

class SharePlugin(private val ctx: PluginContext, private val sink: ShareSink) : Plugin {
    /** Downloads run one at a time, in the order the PC announced them. */
    private val downloadLock = Mutex()
    private val uploadLock = Mutex()

    override suspend fun onPacket(packet: NetworkPacket): Boolean {
        when {
            packet.type == PacketTypes.SHARE_REQUEST_UPDATE -> Unit // batch totals; per-file progress is enough
            packet.hasPayload || packet.has("filename") -> receiveFile(packet)
            packet.has("text") -> sink.onText(ctx.device, packet.string("text").orEmpty())
            packet.has("url") -> sink.onUrl(ctx.device, packet.string("url").orEmpty())
        }
        return true
    }

    private fun receiveFile(packet: NetworkPacket) {
        val name = sanitizeFileName(packet.string("filename") ?: "file-${System.currentTimeMillis()}")
        val size = packet.payloadSize
        ctx.scope.launch(Dispatchers.IO) {
            downloadLock.withLock {
                val id = Transfers.start(ctx.device, name, incoming = true, total = size)
                val target = sink.createFile(ctx.device, name, size, packet.long("lastModified"))
                if (target == null) {
                    Transfers.finish(id, ok = false)
                    return@withLock
                }
                try {
                    val input = ctx.device.openPayload(packet) ?: error("no payload")
                    input.use { it.copyWithProgress(target.output) { done -> Transfers.progress(id, done) } }
                    target.output.close()
                    Transfers.finish(id, ok = true, location = target.complete())
                } catch (e: Exception) {
                    VLog.w(TAG, "receiving $name failed: ${e.message}")
                    target.abort()
                    Transfers.finish(id, ok = false)
                }
            }
        }
    }

    /** Sends files one after another. Returns how many arrived. */
    suspend fun sendFiles(files: List<OutgoingFile>, open: Boolean = false): Int = withContext(Dispatchers.IO) {
        uploadLock.withLock {
            val total = files.sumOf { it.size.coerceAtLeast(0) }
            ctx.send(packet(PacketTypes.SHARE_REQUEST_UPDATE) {
                put("numberOfFiles", files.size)
                put("totalPayloadSize", total)
            })
            var sent = 0
            for (file in files) {
                val id = Transfers.start(ctx.device, file.name, incoming = false, total = file.size)
                val packet = packet(PacketTypes.SHARE_REQUEST) {
                    put("filename", file.name)
                    file.lastModified?.let { put("lastModified", it) }
                    put("open", open)
                    put("numberOfFiles", files.size)
                    put("totalPayloadSize", total)
                }
                val ok = runCatching {
                    ctx.device.sendWithPayload(packet, PayloadSource(file.size, file.open), timeoutMs = 60_000) { done ->
                        Transfers.progress(id, done)
                    }
                }.onFailure { VLog.w(TAG, "sending ${file.name} failed: ${it.message}") }.getOrDefault(false)
                Transfers.finish(id, ok)
                if (ok) sent++
            }
            sent
        }
    }

    suspend fun sendText(text: String) = ctx.send(packet(PacketTypes.SHARE_REQUEST) { put("text", text) })

    /** The PC opens it in its default browser/handler. */
    suspend fun sendUrl(url: String) = ctx.send(packet(PacketTypes.SHARE_REQUEST) { put("url", url) })

    companion object {
        const val ID = "share"
        private const val TAG = "Share"

        fun spec(sink: ShareSink) = PluginSpec(
            ID,
            incoming = setOf(PacketTypes.SHARE_REQUEST, PacketTypes.SHARE_REQUEST_UPDATE),
            outgoing = setOf(PacketTypes.SHARE_REQUEST),
        ) { SharePlugin(it, sink) }

        /** Peers choose the name; never let it escape the destination folder. */
        fun sanitizeFileName(name: String): String {
            val base = name.substringAfterLast('/').substringAfterLast('\\').replace(Regex("[\\u0000-\\u001f]"), "").trim()
            return base.trimStart('.').ifBlank { "file-${System.currentTimeMillis()}" }.take(200)
        }
    }
}
