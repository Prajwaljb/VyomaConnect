package dev.vyoma.connect.protocol.device

import dev.vyoma.connect.protocol.DeviceIdentity
import dev.vyoma.connect.protocol.NetworkPacket
import dev.vyoma.connect.protocol.PacketTypes
import dev.vyoma.connect.protocol.VLog
import dev.vyoma.connect.protocol.crypto.Certificates
import dev.vyoma.connect.protocol.link.DeviceLink
import dev.vyoma.connect.protocol.link.PayloadSource
import dev.vyoma.connect.protocol.link.TransportKind
import dev.vyoma.connect.protocol.packet
import dev.vyoma.connect.protocol.plugin.Plugin
import dev.vyoma.connect.protocol.plugin.PluginContext
import dev.vyoma.connect.protocol.plugin.PluginSpec
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.InputStream
import java.security.cert.X509Certificate
import java.util.concurrent.CopyOnWriteArrayList
import kotlinx.serialization.json.put

enum class PairState { UNPAIRED, REQUESTED_BY_US, REQUESTED_BY_PEER, PAIRED }

data class DeviceState(
    val identity: DeviceIdentity,
    val pairState: PairState,
    val transports: Set<TransportKind>,
    /** 8-char code to compare on both screens while a pairing request is pending. */
    val verificationKey: String? = null,
    /** Ids of the plugins currently loaded; empty while unpaired or offline. */
    val plugins: Set<String> = emptySet(),
) {
    val isReachable: Boolean get() = transports.isNotEmpty()
    val isPaired: Boolean get() = pairState == PairState.PAIRED
}

/** Services a [Device] needs from its owner, so it stays testable without the whole hub. */
interface DeviceEnvironment {
    val localCertificate: X509Certificate
    val trustStore: TrustStore
    val pluginSpecs: List<PluginSpec>
    val scope: CoroutineScope
    fun onPairRequest(device: Device)
}

/**
 * One peer: its links, pairing state machine, and loaded plugins.
 * All state changes run on a single-threaded dispatcher, so no locks are needed.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class Device internal constructor(
    initialIdentity: DeviceIdentity,
    paired: Boolean,
    private val env: DeviceEnvironment,
) {
    val id: String = initialIdentity.deviceId

    private val serial = Dispatchers.Default.limitedParallelism(1)
    private val links = CopyOnWriteArrayList<DeviceLink>()

    /** Where we currently reach this device from; empty when offline. */
    val addresses: List<java.net.InetAddress> get() = links.filter { !it.isClosed }.map { it.remoteAddress }
    private val _state = MutableStateFlow(
        DeviceState(initialIdentity, if (paired) PairState.PAIRED else PairState.UNPAIRED, emptySet()),
    )
    val state: StateFlow<DeviceState> = _state.asStateFlow()

    private var pairingTimestamp: Long? = null
    private var pairTimeout: Job? = null
    private var pluginScope: CoroutineScope? = null
    private val plugins = mutableMapOf<String, Plugin>()
    private val packetRoutes = mutableMapOf<String, Plugin>()

    val name: String get() = _state.value.identity.deviceName
    val identity: DeviceIdentity get() = _state.value.identity
    val peerCertificate: X509Certificate?
        get() = links.firstOrNull()?.peerCertificate ?: env.trustStore.get(id)?.certificate

    @Suppress("UNCHECKED_CAST")
    fun <T : Plugin> plugin(id: String): T? = plugins[id] as? T

    // ---- links ----------------------------------------------------------------------------

    internal suspend fun addLink(link: DeviceLink) = withContext(serial) {
        // A reconnect on the same transport means the old socket is dead or half-open; drop it.
        links.filter { it.transport == link.transport }.forEach { it.close() }
        links.removeAll { it.isClosed }
        links.add(link)
        _state.update { it.copy(identity = link.identity, transports = transports()) }
        if (isPaired()) {
            // Peer may have renamed itself; keep the offline listing current.
            env.trustStore.get(id)?.let {
                env.trustStore.put(
                    it.copy(
                        name = link.identity.deviceName, type = link.identity.deviceType,
                        lastAddress = link.remoteAddress.hostAddress, lastPort = link.identity.tcpPort ?: it.lastPort,
                    ),
                )
            }
        }
        reloadPlugins()
        env.scope.launch {
            link.receive { packet -> withContext(serial) { handle(packet, link) } }
            withContext(serial) { onLinkClosed(link) }
        }
    }

    private suspend fun onLinkClosed(link: DeviceLink) {
        links.remove(link)
        _state.update { it.copy(transports = transports()) }
        if (links.isEmpty()) {
            if (_state.value.pairState == PairState.REQUESTED_BY_PEER || _state.value.pairState == PairState.REQUESTED_BY_US) {
                setPairState(PairState.UNPAIRED)
            }
            stopPlugins()
        }
    }

    private fun transports(): Set<TransportKind> = links.filterNot { it.isClosed }.mapTo(mutableSetOf()) { it.transport }

    /** Sends over the best live link: USB, then LAN, then Bluetooth. */
    suspend fun send(packet: NetworkPacket): Boolean {
        val ordered = links.filterNot { it.isClosed }.sortedBy { it.transport.ordinal }
        for (link in ordered) if (link.send(packet)) return true
        return false
    }

    /**
     * Sends [packet] with a binary payload and streams it; suspends until the peer has
     * downloaded it all (or [timeoutMs] passes without it connecting). Call off the device dispatcher.
     */
    suspend fun sendWithPayload(
        packet: NetworkPacket,
        source: PayloadSource,
        timeoutMs: Int = 30_000,
        onProgress: (Long) -> Unit = {},
    ): Boolean {
        val link = links.filterNot { it.isClosed }.sortedBy { it.transport.ordinal }.firstOrNull { it.payloads != null } ?: return false
        return link.payloads!!.send(link, packet, source, timeoutMs, onProgress)
    }

    /** Opens the payload announced by an incoming [packet]; the caller must close it. */
    suspend fun openPayload(packet: NetworkPacket): InputStream? {
        if (!packet.hasPayload) return null
        val link = links.filterNot { it.isClosed }.sortedBy { it.transport.ordinal }.firstOrNull { it.payloads != null } ?: return null
        return link.payloads!!.open(link, packet)
    }

    fun disconnect() = links.forEach { it.close() }

    // ---- pairing --------------------------------------------------------------------------

    suspend fun requestPairing() = withContext(serial) {
        if (_state.value.pairState == PairState.PAIRED) return@withContext
        val ts = System.currentTimeMillis() / 1000
        pairingTimestamp = ts
        if (send(packet(PacketTypes.PAIR) { put("pair", true); put("timestamp", ts) })) {
            setPairState(PairState.REQUESTED_BY_US)
            armPairTimeout()
        }
    }

    suspend fun acceptPairing() = withContext(serial) {
        if (_state.value.pairState != PairState.REQUESTED_BY_PEER) return@withContext
        if (send(packet(PacketTypes.PAIR) { put("pair", true) })) becomePaired()
    }

    suspend fun rejectPairing() = withContext(serial) {
        if (_state.value.pairState != PairState.REQUESTED_BY_PEER) return@withContext
        send(packet(PacketTypes.PAIR) { put("pair", false) })
        setPairState(PairState.UNPAIRED)
    }

    suspend fun unpair() = withContext(serial) {
        send(packet(PacketTypes.PAIR) { put("pair", false) })
        becomeUnpaired()
    }

    private suspend fun handlePair(packet: NetworkPacket) {
        val wantsPair = packet.bool("pair") ?: return
        val current = _state.value.pairState
        if (!wantsPair) {
            VLog.i(TAG, "$name: unpaired/rejected by peer")
            becomeUnpaired()
            return
        }
        when (current) {
            PairState.REQUESTED_BY_US -> becomePaired()
            // Peer lost its trust (reinstall, cleared data) but we still trust it: re-confirm.
            PairState.PAIRED -> send(packet(PacketTypes.PAIR) { put("pair", true) })
            PairState.UNPAIRED, PairState.REQUESTED_BY_PEER -> {
                pairingTimestamp = packet.long("timestamp")
                setPairState(PairState.REQUESTED_BY_PEER)
                armPairTimeout()
                env.onPairRequest(this)
            }
        }
    }

    private suspend fun becomePaired() {
        val cert = links.firstOrNull()?.peerCertificate ?: return
        val identity = _state.value.identity
        val link = links.firstOrNull()
        env.trustStore.put(TrustedDevice(id, identity.deviceName, identity.deviceType, cert, link?.remoteAddress?.hostAddress, identity.tcpPort))
        setPairState(PairState.PAIRED)
        VLog.i(TAG, "$name: paired")
        reloadPlugins()
    }

    private fun becomeUnpaired() {
        env.trustStore.remove(id)
        setPairState(PairState.UNPAIRED)
        stopPlugins()
    }

    private fun setPairState(state: PairState) {
        pairTimeout?.cancel()
        if (state != PairState.REQUESTED_BY_PEER && state != PairState.REQUESTED_BY_US) pairingTimestamp = null
        val key = if (state == PairState.REQUESTED_BY_US || state == PairState.REQUESTED_BY_PEER) {
            links.firstOrNull()?.let { Certificates.verificationKey(env.localCertificate, it.peerCertificate, pairingTimestamp) }
        } else {
            null
        }
        _state.update { it.copy(pairState = state, verificationKey = key) }
    }

    private fun armPairTimeout() {
        pairTimeout?.cancel()
        pairTimeout = env.scope.launch(serial) {
            delay(PAIR_TIMEOUT_MS)
            val s = _state.value.pairState
            if (s == PairState.REQUESTED_BY_US || s == PairState.REQUESTED_BY_PEER) {
                VLog.i(TAG, "$name: pairing timed out")
                setPairState(PairState.UNPAIRED)
            }
        }
    }

    private fun isPaired() = _state.value.pairState == PairState.PAIRED

    // ---- packets & plugins ----------------------------------------------------------------

    private var warnedUnpaired = false

    private suspend fun handle(packet: NetworkPacket, link: DeviceLink) {
        when {
            packet.type == PacketTypes.PAIR -> handlePair(packet)
            // v8 peers may resend identity; capabilities can change (plugins toggled on the desktop).
            packet.type == PacketTypes.IDENTITY -> Unit
            !isPaired() -> {
                // Peer still thinks we're paired; tell it once so its UI stops showing us as paired.
                if (!warnedUnpaired) {
                    warnedUnpaired = true
                    link.send(packet(PacketTypes.PAIR) { put("pair", false) })
                }
            }
            else -> {
                val plugin = packetRoutes[packet.type]
                if (plugin == null) {
                    VLog.d(TAG, "$name: no plugin for ${packet.type}")
                    return
                }
                runCatching { plugin.onPacket(packet) }
                    .onFailure { VLog.e(TAG, "$name: plugin failed on ${packet.type}", it) }
            }
        }
    }

    /** Capabilities the running plugins were created for; null when none are running. */
    private var loadedFor: Pair<Set<String>, Set<String>>? = null

    private suspend fun reloadPlugins() {
        val identity = _state.value.identity
        val caps = identity.incomingCapabilities to identity.outgoingCapabilities
        // An extra link to an already-connected peer (it dialled us back, or another transport came
        // up) must not restart plugins: that drops state peers only send once, like the PC's battery.
        if (isPaired() && links.isNotEmpty() && loadedFor == caps) return
        stopPlugins()
        if (!isPaired() || links.isEmpty()) return
        loadedFor = caps
        val scope = CoroutineScope(env.scope.coroutineContext + SupervisorJob() + serial)
        pluginScope = scope
        for (spec in env.pluginSpecs) {
            if (!spec.appliesTo(identity.incomingCapabilities, identity.outgoingCapabilities)) continue
            val plugin = runCatching { spec.create(PluginContext(this, scope)) }
                .onFailure { VLog.e(TAG, "creating plugin ${spec.id} failed", it) }
                .getOrNull() ?: continue
            plugins[spec.id] = plugin
            spec.incoming.forEach { packetRoutes[it] = plugin }
        }
        for ((pluginId, plugin) in plugins) {
            runCatching { plugin.onStart() }.onFailure { VLog.e(TAG, "starting plugin $pluginId failed", it) }
        }
        _state.update { it.copy(plugins = plugins.keys.toSet()) }
        VLog.i(TAG, "$name: plugins ${plugins.keys.sorted()}")
    }

    private fun stopPlugins() {
        loadedFor = null
        pluginScope?.cancel()
        pluginScope = null
        plugins.values.forEach { runCatching { it.onStop() } }
        plugins.clear()
        packetRoutes.clear()
        _state.update { if (it.plugins.isEmpty()) it else it.copy(plugins = emptySet()) }
    }

    private companion object {
        const val TAG = "Device"
        const val PAIR_TIMEOUT_MS = 30_000L
    }
}
