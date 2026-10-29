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
    )

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
            DatagramSocket().use { socket ->
                socket.broadcast = true
                for (target in config.broadcastTargets()) {
                    runCatching { socket.send(DatagramPacket(payload, payload.size, target, config.udpPort)) }
                        .onFailure { VLog.d(TAG, "UDP to ${target.hostAddress} failed: ${it.message}") }
                }
            }
        }
    }

    /** Connect directly to a peer we learned about out-of-band (mDNS, UDP, manual). */
    fun connectTo(address: InetAddress, port: Int, peer: DeviceIdentity? = null) {
        val key = peer?.deviceId ?: "${address.hostAddress}:$port"
        if (!connecting.add(key)) return
        scope.launch(Dispatchers.IO) {
            try {
                connectOutgoing(address, port, peer)
            } catch (e: Exception) {
                VLog.d(TAG, "connect to ${address.hostAddress}:$port failed: ${e.message}")
            } finally {
                connecting.remove(key)
            }
        }
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
            if (identity.protocolVersion < DeviceIdentity.MIN_PROTOCOL_VERSION) continue
            if (config.isConnected(identity.deviceId)) continue
            val port = identity.tcpPort ?: continue
            connectTo(datagram.address, port, identity)
        }
    }

    private suspend fun connectOutgoing(address: InetAddress, port: Int, peer: DeviceIdentity?) {
        val plain = Socket()
        try {
            plain.connect(InetSocketAddress(address, port), CONNECT_TIMEOUT_MS)
            config.tuneSocket(plain)
            plain.soTimeout = HANDSHAKE_TIMEOUT_MS
            plain.getOutputStream().apply {
                write(localIdentity().toPacket(tcpPort).serialize().toByteArray(Charsets.UTF_8))
                flush()
            }
            // We opened the TCP connection, so we are the TLS server.
            val ssl = tls.wrap(plain, asServer = true)
            finishHandshake(ssl, preTlsIdentity = peer)
        } catch (e: Exception) {
            runCatching { plain.close() }
            throw e
        }
    }

    private suspend fun acceptLoop(server: ServerSocket) {
        while (scope.isActive && !server.isClosed) {
            val plain = try {
                server.accept()
            } catch (e: IOException) {
                if (!server.isClosed) VLog.w(TAG, "accept failed", e)
                break
            }
            scope.launch(Dispatchers.IO) {
                try {
                    config.tuneSocket(plain)
                    plain.soTimeout = HANDSHAKE_TIMEOUT_MS
                    val line = plain.getInputStream().readPacketLine(limit = 64 * 1024)
                        ?: throw IOException("closed before identity")
                    val identity = DeviceIdentity.fromPacket(NetworkPacket.parse(line))
                    if (identity.deviceId == localIdentity().deviceId) throw ProtocolException("Connected to self")
                    if (identity.protocolVersion < DeviceIdentity.MIN_PROTOCOL_VERSION) {
                        throw ProtocolException("Unsupported protocol ${identity.protocolVersion}")
                    }
                    val ssl = tls.wrap(plain, asServer = false)
                    finishHandshake(ssl, identity)
                } catch (e: Exception) {
                    VLog.d(TAG, "incoming from ${plain.inetAddress.hostAddress} rejected: ${e.message}")
                    runCatching { plain.close() }
                }
            }
        }
    }

    private suspend fun finishHandshake(ssl: SSLSocket, preTlsIdentity: DeviceIdentity?) {
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
        VLog.i(TAG, "link up: ${identity.deviceName} (${identity.deviceId}) v${identity.protocolVersion} via ${ssl.inetAddress.hostAddress}")
        onLink(DeviceLink(identity, peerCert, TransportKind.LAN, ssl))
    }

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

    private companion object {
        const val TAG = "LanBackend"
        const val CONNECT_TIMEOUT_MS = 5_000
        const val HANDSHAKE_TIMEOUT_MS = 10_000
    }
}
