package dev.vyoma.connect.protocol.plugin.builtin

import dev.vyoma.connect.protocol.NetworkPacket
import dev.vyoma.connect.protocol.PacketTypes
import dev.vyoma.connect.protocol.packet
import dev.vyoma.connect.protocol.plugin.Plugin
import dev.vyoma.connect.protocol.plugin.PluginContext
import dev.vyoma.connect.protocol.plugin.PluginSpec
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.serialization.json.put

class PingPlugin(private val ctx: PluginContext) : Plugin {
    data class Ping(val deviceId: String, val message: String?)

    override suspend fun onPacket(packet: NetworkPacket): Boolean {
        pings.tryEmit(Ping(ctx.device.id, packet.string("message")))
        return true
    }

    suspend fun ping(message: String? = null) = ctx.send(packet(PacketTypes.PING) {
        if (message != null) put("message", message)
    })

    companion object {
        const val ID = "ping"

        private val pings = MutableSharedFlow<Ping>(extraBufferCapacity = 8, onBufferOverflow = BufferOverflow.DROP_OLDEST)

        /** Pings from every device; the app turns these into notifications. */
        val received: SharedFlow<Ping> = pings.asSharedFlow()

        val spec = PluginSpec(ID, incoming = setOf(PacketTypes.PING), outgoing = setOf(PacketTypes.PING), ::PingPlugin)
    }
}
