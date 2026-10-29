package dev.vyoma.connect.protocol.device

import dev.vyoma.connect.protocol.DeviceIdentity
import dev.vyoma.connect.protocol.DeviceType
import dev.vyoma.connect.protocol.VLog
import dev.vyoma.connect.protocol.crypto.LocalCredentials
import dev.vyoma.connect.protocol.crypto.TlsContext
import dev.vyoma.connect.protocol.link.DeviceLink
import dev.vyoma.connect.protocol.link.LanBackend
import dev.vyoma.connect.protocol.plugin.PluginSpec
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
        config = lanConfig.copy(isConnected = { id -> _devices.value[id]?.state?.value?.isReachable == true }),
        onLink = ::onLink,
    )

    init {
        // Paired devices are listed even while offline.
        _devices.value = trustStore.all().associate { trusted ->
            trusted.deviceId to Device(
                DeviceIdentity(trusted.deviceId, trusted.name, trusted.type, 0, emptySet(), emptySet()),
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
        lan.broadcastIdentity()
        for (trusted in trustStore.all()) {
            if (_devices.value[trusted.deviceId]?.state?.value?.isReachable == true) continue
            val address = trusted.lastAddress ?: continue
            val ip = runCatching { java.net.InetAddress.getByName(address) }.getOrNull() ?: continue
            lan.connectTo(ip, trusted.lastPort ?: 1716)
        }
        // Manual hosts (often Tailscale) are a fallback: dialling one while the PC is already up
        // over Wi-Fi would swap the fast LAN link for the VPN. Names need DNS, so resolve off-thread.
        if (trustStore.all().isNotEmpty() && !anyPairedOffline()) return
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
        lan.connectTo(address, port)
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
        device!!.addLink(link)
    }

    private companion object {
        const val TAG = "DeviceHub"
        const val DEFAULT_PORT = 1716
    }
}
