package dev.vyoma.connect.protocol.plugin.builtin

import dev.vyoma.connect.protocol.NetworkPacket
import dev.vyoma.connect.protocol.packet
import dev.vyoma.connect.protocol.plugin.Plugin
import dev.vyoma.connect.protocol.plugin.PluginContext
import dev.vyoma.connect.protocol.plugin.PluginSpec
import dev.vyoma.connect.protocol.VLog
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
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

    suspend fun requestStats() = ctx.send(packet(STATS_REQUEST))

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
        private const val SCREENSHOT_TIMEOUT_MS = 30_000L
        private const val TAG = "Companion"
        private val IMAGE_FORMATS = setOf("png", "webp", "jpg")
        fun spec(sink: ShareSink) = PluginSpec(
            ID,
            incoming = setOf(STATS, SCREENSHOT),
            outgoing = setOf(STATS_REQUEST, SCREENSHOT_REQUEST),
        ) { CompanionPlugin(it, sink) }
    }
}
