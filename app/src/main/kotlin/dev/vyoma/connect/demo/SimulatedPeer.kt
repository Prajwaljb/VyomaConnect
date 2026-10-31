package dev.vyoma.connect.demo

import dev.vyoma.connect.protocol.DeviceIdentity
import dev.vyoma.connect.protocol.NetworkPacket
import dev.vyoma.connect.protocol.VLog
import dev.vyoma.connect.protocol.crypto.LocalCredentials
import dev.vyoma.connect.protocol.crypto.TlsContext
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.cancelChildren
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.put
import java.io.IOException
import java.io.OutputStream
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.ServerSocket
import java.net.Socket

/**
 * A pretend KDE Connect peer living inside the demo app. It dials the phone's own listener over
 * loopback and speaks the real protocol (identity, TLS, v8 identity inside TLS, payload sockets),
 * so every screen runs the same code it does against a real PC. Nothing leaves the phone.
 */
internal abstract class SimulatedPeer(
    private val credentials: LocalCredentials,
    private val identity: DeviceIdentity,
) {
    private val tls = TlsContext(credentials)
    private val writeLock = Any()
    @Volatile private var out: OutputStream? = null

    val id: String get() = identity.deviceId

    /** Keeps a link to the phone up, redialling whenever it drops (the service stopped, the app restarted). */
    fun run(scope: CoroutineScope, phonePort: () -> Int) = scope.launch(Dispatchers.IO) {
        while (isActive) {
            val port = phonePort()
            // The phone's listener isn't up yet (the hub starts with the service): look again shortly.
            if (port <= 0) { delay(STARTUP_POLL_MS); continue }
            try {
                session(port)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                VLog.d(TAG, "${identity.deviceName}: ${e.message}")
            }
            delay(RETRY_MS)
        }
    }

    private suspend fun session(port: Int) = coroutineScope {
        Socket().use { plain ->
            plain.connect(InetSocketAddress(InetAddress.getLoopbackAddress(), port), CONNECT_TIMEOUT_MS)
            plain.tcpNoDelay = true
            plain.getOutputStream().apply { write(identityLine()); flush() }
            // As in KDE Connect, whoever opened the TCP connection is the TLS server.
            val ssl = tls.wrap(plain, asServer = true)
            ssl.startHandshake()
            ssl.outputStream.apply { write(identityLine()); flush() }
            val reader = ssl.inputStream.bufferedReader(Charsets.UTF_8)
            reader.readLine() ?: return@use // the phone's identity
            out = ssl.outputStream
            VLog.i(TAG, "${identity.deviceName}: linked")
            val session = this
            try {
                synchronized(this@SimulatedPeer) { onConnected(session) }
                while (true) {
                    val line = reader.readLine() ?: break
                    if (line.isBlank()) continue
                    val packet = runCatching { NetworkPacket.parse(line) }.getOrNull() ?: continue
                    runCatching { synchronized(this@SimulatedPeer) { onPacket(packet, session) } }.onFailure { VLog.w(TAG, "${identity.deviceName}: ${packet.type} failed", it) }
                }
            } finally {
                out = null
                coroutineContext.cancelChildren()
            }
        }
    }

    /** The link is up; [session] lives until it drops. This and [onPacket] run holding the peer's lock. */
    protected abstract fun onConnected(session: CoroutineScope)

    protected abstract fun onPacket(packet: NetworkPacket, session: CoroutineScope)

    protected fun send(packet: NetworkPacket) {
        val bytes = packet.serialize().toByteArray(Charsets.UTF_8)
        synchronized(writeLock) {
            runCatching { out?.apply { write(bytes); flush() } }
        }
    }

    /**
     * Sends [packet] with [bytes] attached: listens on a loopback payload port and lets the phone
     * connect. The phone shifts a loopback peer's payload port by +100 (the companion's adb mirror),
     * so the announced port is the real one minus that.
     */
    protected fun sendPayload(session: CoroutineScope, packet: NetworkPacket, bytes: () -> ByteArray) = session.launch(Dispatchers.IO) {
        val data = bytes()
        val server = bindPayloadPort() ?: return@launch
        server.use {
            it.soTimeout = PAYLOAD_TIMEOUT_MS
            send(packet.withPayload(data.size.toLong(), buildJsonObject { put("port", it.localPort - USB_PAYLOAD_SHIFT) }))
            runCatching {
                it.accept().use { plain ->
                    val ssl = tls.wrap(plain, asServer = true)
                    ssl.startHandshake()
                    ssl.outputStream.apply { write(data); flush() }
                    ssl.close()
                }
            }.onFailure { e -> VLog.d(TAG, "payload not collected: ${e.message}") }
        }
    }

    /** Reads (and drops) a payload the phone offers; returns its size, or -1 if it didn't arrive. */
    protected fun receivePayload(packet: NetworkPacket): Long {
        val port = (packet.payloadTransferInfo?.get("port") as? JsonPrimitive)?.intOrNull ?: return -1
        return runCatching {
            Socket().use { plain ->
                plain.connect(InetSocketAddress(InetAddress.getLoopbackAddress(), port), CONNECT_TIMEOUT_MS)
                val ssl = tls.wrap(plain, asServer = false)
                ssl.soTimeout = PAYLOAD_TIMEOUT_MS
                ssl.startHandshake()
                val buffer = ByteArray(64 * 1024)
                var total = 0L
                ssl.inputStream.use { input ->
                    while (packet.payloadSize < 0 || total < packet.payloadSize) {
                        val n = input.read(buffer)
                        if (n < 0) break
                        total += n
                    }
                }
                total
            }
        }.getOrDefault(-1)
    }

    private fun identityLine(): ByteArray {
        val base = identity.toPacket()
        // The companion's marker: which PC it belongs to (DeviceIdentity reads it as companionOf).
        val body = identity.companionOf?.let { JsonObject(base.body + ("vyomaHost" to JsonPrimitive(it))) } ?: base.body
        return NetworkPacket(base.type, body).serialize().toByteArray(Charsets.UTF_8)
    }

    private fun bindPayloadPort(): ServerSocket? {
        for (port in PAYLOAD_PORTS) {
            try {
                return ServerSocket().apply { reuseAddress = true; bind(InetSocketAddress(InetAddress.getLoopbackAddress(), port)) }
            } catch (_: IOException) {
                // taken, try the next
            }
        }
        return null
    }

    private companion object {
        const val TAG = "DemoPeer"
        const val RETRY_MS = 3_000L
        const val STARTUP_POLL_MS = 250L
        const val CONNECT_TIMEOUT_MS = 3_000
        const val PAYLOAD_TIMEOUT_MS = 15_000
        const val USB_PAYLOAD_SHIFT = 100
        /** Announced as 1770..1799: clear of the phone's own listener (1716..1765) and payload ports. */
        val PAYLOAD_PORTS = 1870..1899
    }
}
