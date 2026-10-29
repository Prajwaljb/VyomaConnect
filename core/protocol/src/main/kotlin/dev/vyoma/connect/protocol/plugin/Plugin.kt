package dev.vyoma.connect.protocol.plugin

import dev.vyoma.connect.protocol.NetworkPacket
import dev.vyoma.connect.protocol.device.Device
import kotlinx.coroutines.CoroutineScope

/**
 * Describes a plugin and when it applies. It's loaded for a paired, reachable device
 * when the capabilities overlap: the peer sends something we handle, or handles something we send.
 */
class PluginSpec(
    val id: String,
    val incoming: Set<String>,
    val outgoing: Set<String>,
    val create: (PluginContext) -> Plugin,
) {
    fun appliesTo(peerIncoming: Set<String>, peerOutgoing: Set<String>): Boolean =
        incoming.any { it in peerOutgoing } || outgoing.any { it in peerIncoming }
}

/** What a plugin instance gets to work with. [scope] is cancelled when the plugin unloads. */
class PluginContext(
    val device: Device,
    val scope: CoroutineScope,
) {
    suspend fun send(packet: NetworkPacket): Boolean = device.send(packet)
}

interface Plugin {
    /** Called once the device is paired and reachable. */
    suspend fun onStart() {}

    /** Return true if handled. Only packets whose type is in the spec's [PluginSpec.incoming] are delivered. */
    suspend fun onPacket(packet: NetworkPacket): Boolean

    /** Called when the device becomes unreachable or unpaired. The plugin scope is already cancelled. */
    fun onStop() {}
}
