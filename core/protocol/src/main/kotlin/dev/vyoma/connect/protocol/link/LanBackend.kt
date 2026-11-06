package dev.vyoma.connect.protocol.link

import dev.vyoma.connect.protocol.DeviceIdentity
import dev.vyoma.connect.protocol.NetworkPacket
import dev.vyoma.connect.protocol.ProtocolException
import dev.vyoma.connect.protocol.VLog
import dev.vyoma.connect.protocol.crypto.Certificates
import dev.vyoma.connect.protocol.crypto.TlsContext
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import java.io.IOException
import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.ServerSocket
import java.net.Socket
import java.security.cert.X509Certificate
import java.util.concurrent.ConcurrentHashMap
import javax.net.ssl.SSLSocket

/**
 * KDE Connect LAN transport.
 *
 * Discovery: each side broadcasts its identity over UDP (port 1716). Whoever *receives*
 * a broadcast opens a TCP connection to the sender's `tcpPort`, sends its own identity in
 * plaintext, and then becomes the TLS **server**. The side that accepted the TCP connection
 * reads that identity and becomes the TLS **client**. On protocol v8 both sides then
 * resend their identity inside TLS, and only that copy is trusted.
 */
class LanBackend(
    private val localIdentity: () -> DeviceIdentity,
    private val tls: TlsContext,
    private val scope: CoroutineScope,
    private val config: Config = Config(),
    private val onLink: suspend (DeviceLink) -> Unit,
) {
    data class Config(
        val udpPort: Int = 1716,
        val tcpPorts: IntRange = 1716..1764,
        /** Bind UDP 1716 to hear broadcasts. Off when another KDE Connect owns the port (e.g. tests). */
        val listenUdp: Boolean = true,
        /** Where to send our identity: broadcast addresses plus user-added hosts (Tailscale, hotspot peer). */
        val broadcastTargets: () -> Collection<InetAddress> = { listOf(InetAddress.getByName("255.255.255.255")) },
        /** True if we already hold a live link to this device; avoids reconnect churn. */
        val isConnected: (deviceId: String) -> Boolean = { false },
        /** Platform socket tuning (keep-alive intervals etc.); called on connected sockets. */
        val tuneSocket: (Socket) -> Unit = { it.keepAlive = true; it.tcpNoDelay = true },
        /**
         * Pins an outgoing socket to the right network before it connects or sends. On Android a
         * VPN (e.g. Tailscale) otherwise captures LAN traffic too; LAN peers must go over Wi-Fi.
         */
        val route: (socket: Any, target: InetAddress) -> Unit = { _, _ -> },
        /** User-added PCs (Tailscale, other subnets): dialled directly on refresh. Called off the main thread. */
        val manualHosts: () -> Collection<InetAddress> = { emptyList() },
        /** What carries a link, from its socket's local and remote addresses. */
        val transportOf: (local: InetAddress, remote: InetAddress) -> TransportKind = ::defaultTransport,
        /**
         * True to turn away a new connection with [deviceId] over [transport] before TLS, because the
         * device is already on a better one. Peers keep only their newest link per device, so a
         * second link that completed would replace the better one on their side. [incoming]: they dialled us.
         */
        val refuses: (deviceId: String, transport: TransportKind, incoming: Boolean) -> Boolean = { _, _, _ -> false },
    )

    private val payloads = LanPayloads(tls, config.tuneSocket, config.route)
    private val handshakes = HandshakeGate(MAX_HANDSHAKES, MAX_HANDSHAKES_PER_ADDRESS)

    @Volatile var tcpPort: Int = -1
        private set

    private var serverSocket: ServerSocket? = null
    private var udpSocket: DatagramSocket? = null
    private val jobs = mutableListOf<Job>()
    private val connecting = ConcurrentHashMap.newKeySet<String>()

    fun start() {
        val server = bindFirstFree(config.tcpPorts)
        serverSocket = server
        tcpPort = server.localPort
        jobs += scope.launch(Dispatchers.IO) { acceptLoop(server) }
        if (config.listenUdp) {
            val udp = DatagramSocket(null).apply {
                reuseAddress = true
                broadcast = true
                bind(InetSocketAddress(config.udpPort))
            }
            udpSocket = udp
            jobs += scope.launch(Dispatchers.IO) { udpLoop(udp) }
        }
        VLog.i(TAG, "LAN backend listening on tcp/$tcpPort" + if (config.listenUdp) ", udp/${config.udpPort}" else "")
    }

    fun stop() {
        jobs.forEach { it.cancel() }
        jobs.clear()
        runCatching { serverSocket?.close() }
        runCatching { udpSocket?.close() }
        serverSocket = null
        udpSocket = null
    }

    /** Announces us on the network. Cheap; call on start, on network change, and on user refresh. */
    fun broadcastIdentity() {
        val payload = localIdentity().toPacket(tcpPort).serialize().toByteArray(Charsets.UTF_8)
        scope.launch(Dispatchers.IO) {
            // One socket per target: each may need a different network (Wi-Fi vs VPN).
            for (target in config.broadcastTargets()) {
                runCatching {
                    DatagramSocket().use { socket ->
                        socket.broadcast = true
                        config.route(socket, target)
                        socket.send(DatagramPacket(payload, payload.size, target, config.udpPort))
                    }
                }.onFailure { VLog.d(TAG, "UDP to ${target.hostAddress} failed: ${it.message}") }
            }
        }
    }

    /**
     * Connect directly to a peer we learned about out-of-band (mDNS, UDP, manual). [expectedId] is
     * who we think is there (a paired device's last address), for [Config.refuses].
     */
    fun connectTo(address: InetAddress, port: Int, peer: DeviceIdentity? = null, expectedId: String? = peer?.deviceId) {
        val key = expectedId ?: "${address.hostAddress}:$port"
        if (!connecting.add(key)) return
        scope.launch(Dispatchers.IO) {
            try {
                connectOutgoing(address, port, peer, expectedId)
            } catch (e: Exception) {
                VLog.d(TAG, "connect to ${address.hostAddress}:$port failed: ${e.message}")
            } finally {
                connecting.remove(key)
            }
        }
    }

    /**
     * The same link as [connectTo], through a local [port] that carries it over another medium
     * ([transport], e.g. a Bluetooth tunnel). TLS stays end to end; such a link has no payload sockets.
     */
    fun connectTunnel(port: Int, expectedId: String, transport: TransportKind) {
        if (!connecting.add(expectedId)) return
        scope.launch(Dispatchers.IO) {
            try {
                connectOutgoing(InetAddress.getLoopbackAddress(), port, null, expectedId, transport)
            } catch (e: Exception) {
                VLog.d(TAG, "$transport tunnel to $expectedId failed: ${e.message}")
            } finally {
                connecting.remove(expectedId)
            }
        }
    }

    /** The other way round: a peer dials us through [plain], which carries it over [transport]. */
    fun acceptTunnel(plain: Socket, transport: TransportKind) {
        scope.launch(Dispatchers.IO) { acceptOn(plain, transport) }
    }

    private fun udpLoop(udp: DatagramSocket) {
        val buffer = ByteArray(64 * 1024)
        while (scope.isActive && !udp.isClosed) {
            val datagram = DatagramPacket(buffer, buffer.size)
            try {
                udp.receive(datagram)
            } catch (e: IOException) {
                if (!udp.isClosed) VLog.w(TAG, "UDP receive failed", e)
                break
            }
            val identity = try {
                DeviceIdentity.fromPacket(NetworkPacket.parse(String(datagram.data, 0, datagram.length, Charsets.UTF_8)))
            } catch (e: ProtocolException) {
                continue
            }
            if (identity.deviceId == localIdentity().deviceId) continue
            // Another KDE Connect app on this same device (e.g. the official one during migration).
            if (isLocalAddress(datagram.address)) continue
            if (identity.protocolVersion < DeviceIdentity.MIN_PROTOCOL_VERSION) continue
            if (config.isConnected(identity.deviceId)) continue
            val port = identity.tcpPort ?: continue
            // Anyone can send these, so they don't get to aim us at any port or pile up our dials.
            if (port !in PEER_PORTS || connecting.size >= MAX_DISCOVERY_DIALS) continue
            connectTo(datagram.address, port, identity)
        }
    }

    private suspend fun connectOutgoing(address: InetAddress, port: Int, peer: DeviceIdentity?, expectedId: String?, tunnel: TransportKind? = null) {
        val plain = Socket()
        val deadline = handshakeDeadline(plain)
        try {
            config.route(plain, address)
            plain.connect(InetSocketAddress(address, port), CONNECT_TIMEOUT_MS)
            if (expectedId != null && config.refuses(expectedId, tunnel ?: config.transportOf(plain.localAddress, plain.inetAddress), false)) {
                throw ProtocolException("already linked over a better transport")
            }
            config.tuneSocket(plain)
            plain.soTimeout = HANDSHAKE_TIMEOUT_MS
            plain.getOutputStream().apply {
                write(localIdentity().toPacket(tcpPort).serialize().toByteArray(Charsets.UTF_8))
                flush()
            }
            // We opened the TCP connection, so we are the TLS server.
            val ssl = tls.wrap(plain, asServer = true)
            finishHandshake(ssl, deadline, preTlsIdentity = peer, tunnel = tunnel)
        } catch (e: Exception) {
            runCatching { plain.close() }
            throw e
        } finally {
            deadline.cancel()
        }
    }

    /** The read timeout restarts with every byte, so a peer trickling them needs a limit on the whole. */
    private fun handshakeDeadline(plain: Socket): Job = scope.launch {
        delay(HANDSHAKE_DEADLINE_MS)
        runCatching { plain.close() }
    }

    private suspend fun acceptLoop(server: ServerSocket) {
        while (scope.isActive && !server.isClosed) {
            val plain = try {
                server.accept()
            } catch (e: IOException) {
                if (!server.isClosed) VLog.w(TAG, "accept failed", e)
                break
            }
            val from = plain.inetAddress
            if (!handshakes.enter(from)) {
                runCatching { plain.close() }
                continue
            }
            scope.launch(Dispatchers.IO) {
                try {
                    acceptOn(plain, null)
                } finally {
                    handshakes.leave(from)
                }
            }
        }
    }

    private suspend fun acceptOn(plain: Socket, tunnel: TransportKind?) {
        val deadline = handshakeDeadline(plain)
        try {
            config.tuneSocket(plain)
            plain.soTimeout = HANDSHAKE_TIMEOUT_MS
            val line = plain.getInputStream().readPacketLine(limit = 64 * 1024)
                ?: throw IOException("closed before identity")
            val hello = NetworkPacket.parse(line)
            val identity = DeviceIdentity.fromPacket(hello)
            if (identity.deviceId == localIdentity().deviceId) throw ProtocolException("Connected to self")
            // Another KDE Connect app on this phone. A companion's call through an adb forward also
            // comes from loopback; TLS and its pin still decide whether it gets anywhere.
            if (tunnel == null && config.listenUdp && !DeviceIdentity.helloFromCompanion(hello) && isLocalAddress(plain.inetAddress)) {
                throw ProtocolException("Peer runs on this device")
            }
            if (identity.protocolVersion < DeviceIdentity.MIN_PROTOCOL_VERSION) {
                throw ProtocolException("Unsupported protocol ${identity.protocolVersion}")
            }
            if (config.refuses(identity.deviceId, tunnel ?: config.transportOf(plain.localAddress, plain.inetAddress), true)) {
                throw ProtocolException("already linked over a better transport")
            }
            val ssl = tls.wrap(plain, asServer = false)
            finishHandshake(ssl, deadline, identity, tunnel)
        } catch (e: Exception) {
            VLog.d(TAG, "incoming from ${plain.inetAddress.hostAddress} rejected: ${e.message}")
            runCatching { plain.close() }
        } finally {
            deadline.cancel()
        }
    }

    private suspend fun finishHandshake(ssl: SSLSocket, deadline: Job, preTlsIdentity: DeviceIdentity?, tunnel: TransportKind? = null) {
        ssl.startHandshake()
        val peerCert = ssl.session.peerCertificates.first() as X509Certificate
        val certId = Certificates.commonName(peerCert)

        // v8: both sides resend identity inside TLS. With no pre-TLS identity (manual/mDNS
        // connect) we assume v8; a v7 peer would never answer and the handshake times out.
        val localV8 = localIdentity().protocolVersion >= 8
        var identity = preTlsIdentity
        val expectTlsIdentity = localV8 && (preTlsIdentity == null || preTlsIdentity.protocolVersion >= 8)
        if (expectTlsIdentity) {
            ssl.outputStream.apply {
                write(localIdentity().toPacket(tcpPort).serialize().toByteArray(Charsets.UTF_8))
                flush()
            }
        }
        if (expectTlsIdentity) {
            val line = ssl.inputStream.readPacketLine(limit = 64 * 1024) ?: throw IOException("closed during identity")
            val tlsIdentity = DeviceIdentity.fromPacket(NetworkPacket.parse(line))
            if (preTlsIdentity != null && tlsIdentity.deviceId != preTlsIdentity.deviceId) {
                throw ProtocolException("Device id changed during handshake")
            }
            identity = tlsIdentity
        }
        if (identity == null) throw ProtocolException("No identity from peer")
        if (identity.protocolVersion >= 8 && certId != identity.deviceId) {
            throw ProtocolException("Certificate CN '$certId' does not match device id '${identity.deviceId}'")
        }
        ssl.soTimeout = 0
        deadline.cancel()
        VLog.i(TAG, "link up: ${identity.deviceName} (${identity.deviceId}) v${identity.protocolVersion} via ${ssl.inetAddress.hostAddress}")
        onLink(DeviceLink(identity, peerCert, tunnel ?: config.transportOf(ssl.localAddress, ssl.inetAddress), ssl, payloads.takeIf { tunnel == null }))
    }

    private fun isLocalAddress(address: InetAddress): Boolean =
        address.isLoopbackAddress || runCatching { java.net.NetworkInterface.getByInetAddress(address) != null }.getOrDefault(false)

    private fun bindFirstFree(range: IntRange): ServerSocket {
        for (port in range) {
            try {
                return ServerSocket().apply {
                    reuseAddress = true
                    bind(InetSocketAddress(port))
                }
            } catch (_: IOException) {
                // in use, try next
            }
        }
        throw IOException("No free TCP port in $range")
    }

    companion object {
        private const val TAG = "LanBackend"
        private const val CONNECT_TIMEOUT_MS = 5_000
        private const val HANDSHAKE_TIMEOUT_MS = 10_000
        private const val HANDSHAKE_DEADLINE_MS = 20_000L
        /**
         * A handshake blocks a thread of Dispatchers.IO, which has 64 and also runs every link and
         * transfer. Anyone can start one, so these stay well under that.
         */
        private const val MAX_HANDSHAKES = 24
        private const val MAX_HANDSHAKES_PER_ADDRESS = 4
        private const val MAX_DISCOVERY_DIALS = 16
        private val PEER_PORTS = 1716..1764

        /**
         * Phone-side port for `adb reverse tcp:1765 tcp:1716`: the PC's KDE Connect reached over
         * the USB cable. Outside 1716..1764 because our own listener holds those ports.
         */
        const val USB_REVERSE_PORT = 1765

        private val tetherInterface = Regex("^(rndis|ncm|usb)\\d*$")

        /** USB for loopback (an adb reverse tunnel) and USB-tethering interfaces; LAN otherwise. */
        fun defaultTransport(local: InetAddress, remote: InetAddress): TransportKind {
            if (remote.isLoopbackAddress) return TransportKind.USB
            val name = runCatching { java.net.NetworkInterface.getByInetAddress(local)?.name }.getOrNull()
            return if (name != null && tetherInterface.matches(name)) TransportKind.USB else TransportKind.LAN
        }
    }
}

/** Counts incoming handshakes in progress, in all and per address. */
private class HandshakeGate(private val total: Int, private val perAddress: Int) {
    private val running = HashMap<InetAddress, Int>()

    @Synchronized fun enter(from: InetAddress): Boolean {
        val mine = running[from] ?: 0
        if (mine >= perAddress || running.values.sum() >= total) return false
        running[from] = mine + 1
        return true
    }

    @Synchronized fun leave(from: InetAddress) {
        val left = (running[from] ?: return) - 1
        if (left > 0) running[from] = left else running.remove(from)
    }
}
