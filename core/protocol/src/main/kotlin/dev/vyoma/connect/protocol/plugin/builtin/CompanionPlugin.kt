package dev.vyoma.connect.protocol.plugin.builtin

import dev.vyoma.connect.protocol.NetworkPacket
import dev.vyoma.connect.protocol.packet
import dev.vyoma.connect.protocol.plugin.Plugin
import dev.vyoma.connect.protocol.plugin.PluginContext
import dev.vyoma.connect.protocol.plugin.PluginSpec
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/** A PC's vitals as the companion reports them. Missing sensors are null. */
data class PcStats(
    val cpu: Float,
    val memUsed: Long,
    val memTotal: Long,
    val cpuTemp: Float?,
    val gpu: Int?,
    val uptimeSeconds: Long?,
)

/**
 * Talks to the Vyoma companion (desktop/companion). Stats are a lease: the companion streams
 * every 2 s for 12 s after each [requestStats], so callers renew while something is on screen
 * and the PC goes quiet as soon as nobody looks.
 */
class CompanionPlugin(private val ctx: PluginContext) : Plugin {
    private val _stats = MutableStateFlow<PcStats?>(null)
    val stats: StateFlow<PcStats?> = _stats.asStateFlow()

    suspend fun requestStats() = ctx.send(packet(STATS_REQUEST))

    override suspend fun onPacket(packet: NetworkPacket): Boolean {
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
        val spec = PluginSpec(ID, incoming = setOf(STATS), outgoing = setOf(STATS_REQUEST)) { CompanionPlugin(it) }
    }
}
