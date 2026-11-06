package dev.vyoma.connect.protocol.device

import dev.vyoma.connect.protocol.crypto.CompanionProof
import dev.vyoma.connect.protocol.DeviceIdentity
import dev.vyoma.connect.protocol.NetworkPacket
import dev.vyoma.connect.protocol.PacketTypes
import dev.vyoma.connect.protocol.VLog
import dev.vyoma.connect.protocol.crypto.Certificates
import dev.vyoma.connect.protocol.link.DeviceLink
import dev.vyoma.connect.protocol.link.MAX_PACKET_BYTES
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
    /** A Vyoma companion: part of another device (its PC), not a computer of its own. */
    val isCompanion: Boolean get() = identity.companionOf != null
}

/** Services a [Device] needs from its owner, so it stays testable without the whole hub. */
interface DeviceEnvironment {
    val localDeviceId: String
    val localCertificate: X509Certificate
    val trustStore: TrustStore
    val pluginSpecs: List<PluginSpec>
    val scope: CoroutineScope
    fun onPairRequest(device: Device)
    /** [deviceId] stopped being trusted, whichever side ended it. */
    fun onUnpaired(deviceId: String) {}
    /** Whether [deviceId] is linked over the USB tunnel right now. */
    fun usbTunnelUp(deviceId: String): Boolean = false
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
    val addresses: List<java.net.InetAddress> get() = links.filter { !it.isClosed && it.transport != TransportKind.BLUETOOTH }.map { it.remoteAddress }

    /**
     * A UDP socket connected to [port] on this device, over its newest network link and routed as
     * that link is; null when it's only reachable through a tunnel (USB's adb forwards are TCP only).
     */
    fun datagramTo(port: Int): java.net.DatagramSocket? {
        val link = links.lastOrNull { !it.isClosed && !it.remoteAddress.isLoopbackAddress } ?: return null
        val socket = java.net.DatagramSocket()
        return runCatching {
            link.payloads?.route(socket, link.remoteAddress)
            socket.connect(link.remoteAddress, port)
            socket
        }.getOrElse { socket.close(); null }
    }
    private val _state = MutableStateFlow(
        DeviceState(initialIdentity, if (paired) PairState.PAIRED else PairState.UNPAIRED, emptySet()),
    )
    val state: StateFlow<DeviceState> = _state.asStateFlow()

    private var pairingTimestamp: Long? = null
    /** Whose code is on screen while a pairing is pending: the only certificate it can end up trusting. */
    private var pairingCertificate: X509Certificate? = null
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

    /**
     * All of a device's links show one certificate, so what the user checked while pairing is what
     * gets pinned and obeyed. Device ids are public (they're broadcast), so anyone can dial us under
     * this one with their own certificate.
     */
    private fun admits(link: DeviceLink): Boolean {
        env.trustStore.get(id)?.let { return it.certificate == link.peerCertificate }
        pairingCertificate?.let { return it == link.peerCertificate }
        val current = links.firstOrNull { !it.isClosed }?.peerCertificate ?: return true
        if (current != link.peerCertificate) {
            // Nobody is trusted yet, so the newest wins as in any reconnect: refusing here would let
            // a stranger hold a PC's id and keep the real one out.
            links.forEach { it.close() }
            links.clear()
        }
        return true
    }

    internal suspend fun addLink(link: DeviceLink) = withContext(serial) {
        if (!admits(link)) {
            VLog.w(TAG, "$name: refused a link with a different certificate")
            link.close()
            return@withContext
        }
        // A reconnect on the same transport means the old socket is dying or half-open. Close it after
        // a short grace period rather than at once: when both sides dial together the peer has often
        // just sent its first packets (battery, sinks) down the old one.
        links.removeAll { it.isClosed }
        links.filter { it.transport == link.transport }.forEach { old ->
            env.scope.launch { delay(REPLACED_LINK_GRACE_MS); old.close() }
        }
        links.add(link)
        linger?.cancel()
        _state.update { it.copy(identity = link.identity, transports = transports()) }
        if (isPaired()) {
            // Peer may have renamed itself; keep the offline listing current.
            env.trustStore.get(id)?.let {
                // A loopback peer is an adb tunnel: keep the network address for reconnecting later.
                val tunnel = link.remoteAddress.isLoopbackAddress
                env.trustStore.put(
                    it.copy(
                        name = link.identity.deviceName, type = link.identity.deviceType,
                        lastAddress = if (tunnel) it.lastAddress else link.remoteAddress.hostAddress,
                        lastPort = if (tunnel) it.lastPort else link.identity.tcpPort ?: it.lastPort,
                    ),
                )
            }
        }
        reloadPlugins()
        env.scope.launch {
            link.receive(limit = { if (isPaired()) MAX_PACKET_BYTES else UNPAIRED_PACKET_BYTES }) { packet ->
                withContext(serial) { handle(packet, link) }
            }
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
            // Peers often swap links (both sides dial at once and one socket wins), leaving us with
            // none for a moment. Keep plugins through that gap: restarting them loses state peers
            // send only once per connection, like the PC's battery.
            linger?.cancel()
            linger = env.scope.launch(serial) {
                delay(LINGER_MS)
                if (links.isEmpty()) stopPlugins()
            }
        }
    }

    private var linger: Job? = null

    private fun transports(): Set<TransportKind> = links.filterNot { it.isClosed }.mapTo(mutableSetOf()) { it.transport }

    /** Sends over the best live link: USB, then LAN, then Bluetooth. */
    suspend fun send(packet: NetworkPacket): Boolean {
        // Best transport first and, within one, the newest link: older ones are being replaced.
        val ordered = links.filterNot { it.isClosed }.asReversed().sortedBy { it.transport.ordinal }
        for (link in ordered) if (link.send(packet)) return true
        return false
    }

    /** False when the only way there has no payload sockets (a Bluetooth tunnel). */
    val canCarryPayloads: Boolean get() = links.any { !it.isClosed && it.payloads != null }

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
        // A companion links over Wi-Fi only; with its PC's cable tunnel up it can fetch over USB.
        val usb = identity.companionOf?.let(env::usbTunnelUp) == true
        return link.payloads!!.send(link, packet, source, timeoutMs, onProgress, usbMirror = usb)
    }

    /**
     * Opens the payload announced by an incoming [packet]; the caller must close it. The sender
     * listens on every interface, so if the preferred link can't reach it (an adb tunnel without
     * the payload mirrors) another link to the same device is tried.
     */
    suspend fun openPayload(packet: NetworkPacket): InputStream? {
        if (!packet.hasPayload) return null
        var failure: java.io.IOException? = null
        // A companion only links over Wi-Fi, but its PC's USB tunnel mirrors the payload ports too:
        // the cable is far quicker and steadier (it's what keeps the live screen from lagging).
        val host = identity.companionOf
        if (host != null && links.none { it.transport == TransportKind.USB } && env.usbTunnelUp(host)) {
            links.firstOrNull { !it.isClosed && it.payloads != null }?.let { link ->
                try {
                    return link.payloads!!.open(link, packet, java.net.InetAddress.getLoopbackAddress())
                } catch (e: java.io.IOException) {
                    failure = e
                }
            }
        }
        for (link in links.filterNot { it.isClosed }.sortedBy { it.transport.ordinal }.filter { it.payloads != null }) {
            try {
                return link.payloads!!.open(link, packet)
            } catch (e: java.io.IOException) {
                failure = e
            }
        }
        failure?.let { throw it }
        return null
    }

    fun disconnect() = links.forEach { it.close() }

    /** How long the oldest open link over [transport] has been up, or null if there's none. */
    fun linkAge(transport: TransportKind): Long? =
        links.filter { it.transport == transport && !it.isClosed }.minOfOrNull { it.since }?.let { System.nanoTime() / 1_000_000 - it }

    /** Closes the links over [transport] (the cable was pulled: don't wait for them to time out). */
    fun disconnect(transport: TransportKind) = links.filter { it.transport == transport }.forEach { it.close() }

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
        val cert = pairingCertificate ?: return@withContext
        // Sending lets go of this dispatcher: the request can end meanwhile.
        if (send(packet(PacketTypes.PAIR) { put("pair", true) }) && pairingCertificate === cert) becomePaired(cert)
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

    private suspend fun handlePair(packet: NetworkPacket, link: DeviceLink) {
        val wantsPair = packet.bool("pair") ?: return
        val current = _state.value.pairState
        if (!wantsPair) {
            VLog.i(TAG, "$name: unpaired/rejected by peer")
            becomeUnpaired()
            return
        }
        when (current) {
            PairState.REQUESTED_BY_US -> pairingCertificate?.let { becomePaired(it) }
            // Peer lost its trust (reinstall, cleared data) but we still trust it: re-confirm.
            PairState.PAIRED -> send(packet(PacketTypes.PAIR) { put("pair", true) })
            PairState.UNPAIRED, PairState.REQUESTED_BY_PEER -> {
                if (isProvenCompanion(packet, link.peerCertificate)) {
                    VLog.i(TAG, "$name: companion of a trusted PC, pairing without a prompt")
                    // Sending lets go of this dispatcher, and an unpaired id takes the newest link:
                    // hold on to the certificate that was proved, so only that one can be pinned.
                    val cert = link.peerCertificate
                    pairingCertificate = cert
                    if (send(packet(PacketTypes.PAIR) { put("pair", true) }) && pairingCertificate === cert) becomePaired(cert)
                    else if (pairingCertificate === cert && _state.value.pairState == PairState.UNPAIRED) pairingCertificate = null
                    return
                }
                val timestamp = packet.long("timestamp")
                if (!timestampAllowed(timestamp)) {
                    VLog.w(TAG, "$name: refused a pairing request: its clock is off, or it gave no time")
                    send(packet(PacketTypes.PAIR) { put("pair", false) })
                    setPairState(PairState.UNPAIRED)
                    return
                }
                pairingTimestamp = timestamp
                setPairState(PairState.REQUESTED_BY_PEER)
                armPairTimeout()
                env.onPairRequest(this)
            }
        }
    }

    /**
     * The time goes into the code the user compares. Left free, a sender could try timestamps until
     * the 8 characters match what it needs. Protocol 7 has no timestamp at all.
     */
    private fun timestampAllowed(timestamp: Long?): Boolean {
        if (timestamp == null) return _state.value.identity.protocolVersion < 8
        return kotlin.math.abs(System.currentTimeMillis() / 1000 - timestamp) <= PAIR_CLOCK_SKEW_S
    }

    /** A pair request signed by the key of the PC this peer claims to belong to (see [CompanionProof]). */
    private fun isProvenCompanion(packet: NetworkPacket, ownCert: X509Certificate): Boolean {
        val host = _state.value.identity.companionOf ?: return false
        if (packet.string("vyomaHost") != host) return false
        val hostCert = env.trustStore.get(host)?.certificate ?: return false
        val ts = packet.long("timestamp") ?: return false
        val proof = packet.string("vyomaHostProof") ?: return false
        return CompanionProof.verify(hostCert, id, ownCert, env.localDeviceId, ts, proof)
    }

    private suspend fun becomePaired(cert: X509Certificate) {
        val identity = _state.value.identity
        val link = links.firstOrNull { it.peerCertificate == cert }
        env.trustStore.put(
            TrustedDevice(id, identity.deviceName, identity.deviceType, cert, link?.remoteAddress?.hostAddress, identity.tcpPort, identity.companionOf),
        )
        setPairState(PairState.PAIRED)
        VLog.i(TAG, "$name: paired")
        reloadPlugins()
    }

    private fun becomeUnpaired() {
        val wasTrusted = env.trustStore.get(id) != null
        env.trustStore.remove(id)
        setPairState(PairState.UNPAIRED)
        stopPlugins()
        if (wasTrusted) env.onUnpaired(id)
    }

    private fun setPairState(state: PairState) {
        pairTimeout?.cancel()
        if (state != PairState.REQUESTED_BY_PEER && state != PairState.REQUESTED_BY_US) pairingTimestamp = null
        val pending = state == PairState.REQUESTED_BY_US || state == PairState.REQUESTED_BY_PEER
        pairingCertificate = if (pending) links.firstOrNull { !it.isClosed }?.peerCertificate else null
        val key = if (pending) {
            pairingCertificate?.let { Certificates.verificationKey(env.localCertificate, it, pairingTimestamp) }
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
        // Read just before a refused or replaced link was closed.
        if (link.isClosed) return
        when {
            packet.type == PacketTypes.PAIR -> handlePair(packet, link)
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
        if (isPaired() && links.isNotEmpty() && loadedFor == caps) {
            for ((pluginId, plugin) in plugins) {
                runCatching { plugin.onLinkAdded() }.onFailure { VLog.e(TAG, "re-syncing plugin $pluginId failed", it) }
            }
            return
        }
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
        const val PAIR_CLOCK_SKEW_S = 1800L
        /** Before pairing a peer has nothing to say that takes more; anyone can get this far. */
        const val UNPAIRED_PACKET_BYTES = 64 * 1024
        const val LINGER_MS = 10_000L
        const val REPLACED_LINK_GRACE_MS = 2_000L
    }
}
