package dev.vyoma.connect.protocol.device

import dev.vyoma.connect.protocol.DeviceIdentity
import dev.vyoma.connect.protocol.DeviceType
import dev.vyoma.connect.protocol.VLog
import dev.vyoma.connect.protocol.crypto.LocalCredentials
import dev.vyoma.connect.protocol.crypto.TlsContext
import dev.vyoma.connect.protocol.link.DeviceLink
import dev.vyoma.connect.protocol.link.LanBackend
import dev.vyoma.connect.protocol.plugin.PluginSpec
import dev.vyoma.connect.protocol.link.TransportKind
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import java.security.cert.X509Certificate

sealed interface HubEvent {
    data class PairRequested(val device: Device) : HubEvent
    /** A known device presented a different certificate than the one pinned at pairing. */
    data class CertificateMismatch(val deviceId: String, val name: String) : HubEvent
}

/**
 * Owns this device's identity, all known peers, and the transports.
 * Platform code (Android service, tests) creates one, then starts it.
 */
class DeviceHub(
    private val credentials: LocalCredentials,
    override val trustStore: TrustStore,
    override val pluginSpecs: List<PluginSpec>,
    override val scope: CoroutineScope,
    deviceName: String,
    private val deviceType: DeviceType,
    private val lanConfig: LanBackend.Config = LanBackend.Config(),
) : DeviceEnvironment {

    @Volatile var deviceName: String = DeviceIdentity.sanitizeName(deviceName)
        set(value) { field = DeviceIdentity.sanitizeName(value) }

    val tls = TlsContext(credentials)
    override val localCertificate: X509Certificate get() = credentials.certificate
    override val localDeviceId: String get() = credentials.deviceId

    fun localIdentity() = DeviceIdentity(
        deviceId = credentials.deviceId,
        deviceName = deviceName,
        deviceType = deviceType,
        protocolVersion = DeviceIdentity.PROTOCOL_VERSION,
        incomingCapabilities = pluginSpecs.flatMapTo(sortedSetOf()) { it.incoming },
        outgoingCapabilities = pluginSpecs.flatMapTo(sortedSetOf()) { it.outgoing },
    )

    private val _devices = MutableStateFlow<Map<String, Device>>(emptyMap())
    val devices: StateFlow<Map<String, Device>> = _devices.asStateFlow()

    private val _events = MutableSharedFlow<HubEvent>(extraBufferCapacity = 16, onBufferOverflow = BufferOverflow.DROP_OLDEST)
    val events: SharedFlow<HubEvent> = _events.asSharedFlow()

    val lan = LanBackend(
        localIdentity = ::localIdentity,
        tls = tls,
        scope = scope,
        config = lanConfig.copy(
            isConnected = { id -> _devices.value[id]?.state?.value?.isReachable == true },
            // The cable wins. A PC (kdeconnectd) keeps only its newest connection per device, so a
            // Wi-Fi link completing while we're on USB would knock the cable link out on its side.
            // We never dial Wi-Fi then. The PC only dials a device it thinks is gone, so its Wi-Fi
            // call is a race with a cable link that just came up (refuse it), or a sign that an
            // older tunnel died on its side without us noticing (take it; onLink drops the tunnel).
            refuses = { id, transport, incoming ->
                // Companions never come over the cable (they dial the phone's Wi-Fi address), so
                // holding Wi-Fi back for the tunnel only delayed them after every app start.
                val companion = _devices.value[id]?.identity?.companionOf != null
                if (transport != TransportKind.USB && holdingWifi() && !companion) return@copy true
                val usbAge = if (transport == TransportKind.USB) null else _devices.value[id]?.linkAge(TransportKind.USB)
                usbAge != null && (!incoming || usbAge < FRESH_CABLE_MS)
            },
        ),
        onLink = ::onLink,
    )

    init {
        // Paired devices are listed even while offline.
        _devices.value = trustStore.all().associate { trusted ->
            trusted.deviceId to Device(
                DeviceIdentity(trusted.deviceId, trusted.name, trusted.type, 0, emptySet(), emptySet(), companionOf = trusted.companionOf),
                paired = true,
                env = this,
            )
        }
    }

    fun start() {
        lan.start()
        refresh()
    }

    fun stop() {
        lan.stop()
        _devices.value.values.forEach { it.disconnect() }
    }

    /**
     * Re-announce, and dial paired devices that are offline at their last known address
     * (UDP broadcasts get lost on busy Wi-Fi). Call on network change, refresh, or periodically.
     */
    fun refresh() {
        if (holdingWifi()) return // cablePlugged() refreshes when the head start ends
        lan.broadcastIdentity()
        val trusted = trustStore.all()
        val onCable = onCableAddresses()
        for (trusted in trusted) {
            if (_devices.value[trusted.deviceId]?.state?.value?.isReachable == true) continue
            // Companions have no listener; they always dial us. Their address + 1716 is kdeconnectd.
            if (trusted.companionOf != null) continue
            val address = trusted.lastAddress ?: continue
            if (address in onCable) continue
            val ip = runCatching { java.net.InetAddress.getByName(address) }.getOrNull() ?: continue
            lan.connectTo(ip, trusted.lastPort ?: 1716, expectedId = trusted.deviceId)
        }
        // Manual hosts (often Tailscale) are a fallback: dialling one while the PC is already up
        // over Wi-Fi would swap the fast LAN link for the VPN. Names need DNS, so resolve off-thread.
        if (trusted.isNotEmpty() && !anyPairedOffline()) return
        scope.launch(kotlinx.coroutines.Dispatchers.IO) {
            for (host in runCatching { lanConfig.manualHosts() }.getOrDefault(emptyList())) dial(host, DEFAULT_PORT)
        }
    }

    /**
     * Connects to a peer found out-of-band (mDNS, manual address) unless we already have a
     * link from that address, or to that device id when it's known.
     */
    fun dial(address: java.net.InetAddress, port: Int, deviceId: String? = null) {
        val devices = _devices.value.values
        if (deviceId != null && devices.any { it.id == deviceId && it.state.value.isReachable }) return
        if (devices.any { address in it.addresses }) return
        if (!address.isLoopbackAddress && (holdingWifi() || address.hostAddress in onCableAddresses())) return
        lan.connectTo(address, port, expectedId = deviceId)
    }

    /**
     * The cable is in: hold Wi-Fi back briefly so the USB tunnel connects first. Dialling both at
     * once races, and the PC keeps whichever link finishes last. With no tunnel (a charging-only
     * cable) Wi-Fi just starts a few seconds later.
     */
    fun cablePlugged() {
        wifiHeldUntil = now() + CABLE_HEAD_START_MS
        scope.launch {
            delay(CABLE_HEAD_START_MS)
            refresh()
        }
    }

    /**
     * Network addresses of paired devices that are on the cable right now. Nothing is dialled
     * there over Wi-Fi, under any name (a companion shares its PC's address): kdeconnectd would
     * take the call and drop the cable link.
     */
    private fun onCableAddresses(): Set<String> = trustStore.all()
        .filter { TransportKind.USB in (_devices.value[it.deviceId]?.state?.value?.transports ?: emptySet()) }
        .mapNotNullTo(mutableSetOf()) { it.lastAddress }

    @Volatile private var wifiHeldUntil = 0L
    private fun holdingWifi() = now() < wifiHeldUntil
    private fun now() = System.nanoTime() / 1_000_000

    /** The cable is out: drop USB links now (half-open tunnels can linger) and get back on the network. */
    fun cableUnplugged() {
        wifiHeldUntil = 0
        _devices.value.values.forEach { it.disconnect(TransportKind.USB) }
        scope.launch {
            delay(CABLE_SETTLE_MS) // let the closed links register, so refresh dials what's now offline
            refresh()
        }
    }

    /** True while some paired device isn't connected; the service uses it to keep retrying. */
    fun anyPairedOffline(): Boolean = trustStore.all().any { _devices.value[it.deviceId]?.state?.value?.isReachable != true }

    fun device(id: String): Device? = _devices.value[id]

    /** Drops an unpaired, unreachable device from the list. */
    fun forget(id: String) {
        val device = _devices.value[id] ?: return
        if (device.state.value.isPaired || device.state.value.isReachable) return
        _devices.update { it - id }
    }

    override fun onPairRequest(device: Device) {
        _events.tryEmit(HubEvent.PairRequested(device))
    }

    private suspend fun onLink(link: DeviceLink) {
        val id = link.identity.deviceId
        val pinned = trustStore.get(id)
        if (pinned != null && pinned.certificate != link.peerCertificate) {
            VLog.w(TAG, "${link.identity.deviceName}: certificate changed since pairing; refusing link")
            _events.tryEmit(HubEvent.CertificateMismatch(id, link.identity.deviceName))
            link.close()
            return
        }
        var device: Device? = null
        _devices.update { map ->
            if (map.containsKey(id)) {
                device = map[id]
                map
            } else {
                val created = Device(link.identity, paired = pinned != null, env = this)
                device = created
                map + (id to created)
            }
        }
        // Only reachable with a cable link up if that link is stale (see `refuses`): drop it, or
        // sends would keep going down the dead tunnel.
        if (link.transport != TransportKind.USB) device!!.disconnect(TransportKind.USB)
        device!!.addLink(link)
    }

    private companion object {
        const val TAG = "DeviceHub"
        const val DEFAULT_PORT = 1716
        const val CABLE_SETTLE_MS = 300L
        const val FRESH_CABLE_MS = 10_000L
        const val CABLE_HEAD_START_MS = 4_000L
    }
}
