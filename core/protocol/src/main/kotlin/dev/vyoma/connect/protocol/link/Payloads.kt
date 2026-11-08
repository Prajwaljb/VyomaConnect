package dev.vyoma.connect.protocol.link

import dev.vyoma.connect.protocol.NetworkPacket
import dev.vyoma.connect.protocol.ProtocolException
import dev.vyoma.connect.protocol.VLog
import dev.vyoma.connect.protocol.crypto.TlsContext
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.put
import java.io.Closeable
import java.io.FilterInputStream
import java.io.IOException
import java.io.InputStream
import java.net.InetSocketAddress
import java.net.ServerSocket
import java.net.Socket
import java.security.cert.X509Certificate
import java.util.concurrent.ConcurrentHashMap
import javax.net.ssl.SSLSocket

/** Something to upload: [size] bytes from a fresh stream each time [open] is called (-1 if unknown). */
class PayloadSource(val size: Long, val open: () -> InputStream)

/**
 * Moves a packet's binary payload over its own TLS socket, separate from the packet stream.
 *
 * The sender listens on a port in 1739..1764 and announces it in `payloadTransferInfo`;
 * the receiver connects. As with links, the side that *accepts* is the TLS server here.
 * Either way the peer must present the same certificate as the link it belongs to.
 */
interface PayloadTransport {
    /** Sends [socket]'s traffic to [target] the way this transport's own sockets go (Wi-Fi past a VPN). */
    fun route(socket: Any, target: java.net.InetAddress) {}

    /**
     * Announces [packet] with [source] attached and streams it. Returns true once fully sent.
     * [usbMirror]: the peer (a companion, linked over Wi-Fi) can also reach us through its PC's adb
     * forward, so the announcement names that port too and it fetches over the cable.
     */
    suspend fun send(link: DeviceLink, packet: NetworkPacket, source: PayloadSource, timeoutMs: Int, onProgress: (Long) -> Unit, usbMirror: Boolean = false): Boolean

    /**
     * Connects to the payload announced in [packet]. The stream ends after `payloadSize` bytes.
     * [via] reaches the peer at another address than the link's (the PC's USB tunnel).
     */
    suspend fun open(link: DeviceLink, packet: NetworkPacket, via: java.net.InetAddress? = null): InputStream
}

internal class LanPayloads(
    private val tls: TlsContext,
    private val tuneSocket: (Socket) -> Unit,
    private val route: (Any, java.net.InetAddress) -> Unit,
) : PayloadTransport {
    override fun route(socket: Any, target: java.net.InetAddress) = route.invoke(socket, target)

    override suspend fun send(
        link: DeviceLink,
        packet: NetworkPacket,
        source: PayloadSource,
        timeoutMs: Int,
        onProgress: (Long) -> Unit,
        usbMirror: Boolean,
    ): Boolean = withContext(Dispatchers.IO) {
        val server = bindPayloadPort()
        var ssl: SSLSocket? = null
        closeOnCancel(server, { ssl }) {
            // Over the adb tunnel the PC reaches our port through its forward of it, shifted.
            val port = if (link.remoteAddress.isLoopbackAddress) server.localPort + USB_FORWARD_SHIFT else server.localPort
            val announced = packet.withPayload(source.size, buildJsonObject {
                put("port", port)
                if (usbMirror && !link.remoteAddress.isLoopbackAddress) put("usbPort", server.localPort + USB_FORWARD_SHIFT)
            })
            if (!link.send(announced)) return@closeOnCancel false
            val secure = acceptPeer(server, link.peerCertificate, timeoutMs, packet.type, live = source.size < 0).also { ssl = it }
            VLog.d(TAG, "payload port ${server.localPort} (${packet.type}): peer connected")
            secure.use {
                it.soTimeout = IO_TIMEOUT_MS
                source.open().use { input -> input.copyWithProgress(it.outputStream, onProgress) }
                it.outputStream.flush()
            }
            true
        }
    }

    /**
     * The first connection to prove it's [expected]. Anyone on the network can reach this port, so
     * one that's wrong or stalls is dropped and the wait goes on, for [timeoutMs] in all. Proofs run
     * side by side, each with a time limit, so a few idle connections can't hold up the real one.
     */
    private suspend fun acceptPeer(server: ServerSocket, expected: X509Certificate, timeoutMs: Int, what: String, live: Boolean = false): SSLSocket = coroutineScope {
        val peer = CompletableDeferred<SSLSocket>()
        val proving = ConcurrentHashMap.newKeySet<Socket>()
        val slots = Semaphore(MAX_PROOFS)
        val deadline = System.nanoTime() / 1_000_000 + timeoutMs
        val waiter = launch {
            try {
                while (true) {
                    slots.acquire()
                    server.soTimeout = (deadline - System.nanoTime() / 1_000_000).coerceAtLeast(1).toInt()
                    val plain = server.accept()
                    proving += plain
                    val acceptedAt = System.nanoTime()
                    launch {
                        var wrapped: SSLSocket? = null
                        val limit = launch {
                            delay(PROOF_LIMIT_MS)
                            runCatching { plain.close() }
                        }
                        try {
                            tuneSocket(plain)
                            // A live stream's backlog belongs in its own queue, where the encoder sees it and
                            // lowers the bitrate: Android's send buffer took megabytes, seconds of picture on
                            // slow Wi-Fi, so the encoder never knew and the PC saw it freeze (2026-11-08).
                            if (live) plain.sendBufferSize = LIVE_SEND_BUFFER
                            val secure = tls.wrap(plain, asServer = true).also { wrapped = it }
                            secure.startHandshake()
                            verifyPeer(secure, expected)
                            limit.cancel()
                            proving -= plain // before it's handed over: the rest get closed then
                            if (!peer.complete(secure)) runCatching { secure.close() }
                        } catch (e: Exception) {
                            val ms = (System.nanoTime() - acceptedAt) / 1_000_000
                            VLog.d(TAG, "payload port ${server.localPort} ($what): dropped a connection from ${plain.inetAddress?.hostAddress}:${plain.port} after $ms ms: ${e.message}")
                            proving -= plain
                            runCatching { wrapped?.close() }
                            runCatching { plain.close() }
                        } finally {
                            limit.cancel()
                            slots.release()
                        }
                    }
                }
            } catch (e: IOException) {
                peer.completeExceptionally(e) // out of time, or cancelled
            }
        }
        try {
            peer.await()
        } finally {
            waiter.cancel()
            runCatching { server.close() }
            proving.forEach { runCatching { it.close() } }
        }
    }

    override suspend fun open(link: DeviceLink, packet: NetworkPacket, via: java.net.InetAddress?): InputStream = withContext(Dispatchers.IO) {
        val port = (packet.payloadTransferInfo?.get("port") as? JsonPrimitive)?.let { it.intOrNull ?: it.content.toIntOrNull() }
            ?: throw ProtocolException("Payload without port")
        // Over the adb tunnel the PC's payload port is reached through its mirror on this phone
        // (the companion reverses each one, shifted clear of our own payload ports).
        val host = via ?: link.remoteAddress
        val target = if (host.isLoopbackAddress) port + USB_PAYLOAD_SHIFT else port
        val plain = Socket()
        var secure: SSLSocket? = null
        try {
            route(plain, host)
            plain.connect(InetSocketAddress(host, target), CONNECT_TIMEOUT_MS)
            tuneSocket(plain)
            val ssl = tls.wrap(plain, asServer = false).also { secure = it }
            ssl.soTimeout = IO_TIMEOUT_MS
            ssl.startHandshake()
            verifyPeer(ssl, link.peerCertificate)
            // An open-ended stream (the PC's live screen) goes quiet whenever nothing changes: no read timeout.
            if (packet.payloadSize < 0) ssl.also { it.soTimeout = 0 }.inputStream else BoundedInputStream(ssl.inputStream, packet.payloadSize, ssl)
        } catch (e: Exception) {
            runCatching { secure?.close() }
            runCatching { plain.close() }
            throw e
        }
    }

    private fun verifyPeer(ssl: SSLSocket, expected: X509Certificate) {
        val cert = ssl.session.peerCertificates.firstOrNull() as? X509Certificate
        if (cert != expected) {
            ssl.close()
            throw ProtocolException("Payload peer presented a different certificate")
        }
    }

    private fun bindPayloadPort(): ServerSocket {
        for (port in PAYLOAD_PORTS) {
            val server = ServerSocket()
            try {
                server.reuseAddress = true
                server.bind(InetSocketAddress(port))
                return server
            } catch (_: IOException) {
                // in use (another transfer), try the next one
                runCatching { server.close() }
            }
        }
        throw IOException("No free payload port in $PAYLOAD_PORTS")
    }

    private companion object {
        val PAYLOAD_PORTS = 1739..1764
        /**
         * The companion's `adb reverse tcp:(P + shift) tcp:P` for each payload port: the PC's
         * payloads (album art, files) reach the phone over the cable without colliding with the
         * phone's own [PAYLOAD_PORTS]. Must match PAYLOAD_SHIFT in desktop/companion/src/usb.rs.
         */
        const val USB_PAYLOAD_SHIFT = 100
        /**
         * The other way: the companion's `adb forward tcp:(P + shift) tcp:P`, so the PC reaches
         * the payloads we offer over the cable. Must match FORWARD_SHIFT in desktop/companion/src/usb.rs.
         */
        const val USB_FORWARD_SHIFT = 200
        const val TAG = "LanPayloads"
        const val CONNECT_TIMEOUT_MS = 5_000
        const val MAX_PROOFS = 4
        /** A live stream's socket send buffer (Linux doubles it): ~0.1 s at the mirror's top bitrate. */
        private const val LIVE_SEND_BUFFER = 64 * 1024
        const val PROOF_LIMIT_MS = 5_000L
        const val IO_TIMEOUT_MS = 30_000
    }
}

/** Blocking socket calls don't respond to cancellation, so close the sockets when the coroutine is cancelled. */
private suspend fun <T> closeOnCancel(server: ServerSocket, client: () -> Closeable?, block: suspend () -> T): T = coroutineScope {
    val watchdog = launch {
        try {
            awaitCancellation()
        } finally {
            runCatching { server.close() }
            runCatching { client()?.close() }
        }
    }
    try {
        block()
    } catch (e: java.io.IOException) {
        // Cancelling closes the sockets under the transfer, which then fails: that's the cancel,
        // not an error (it reached the crash handler when a viewer closed, 2026-11-04).
        if (coroutineContext[kotlinx.coroutines.Job]?.isCancelled == true) throw kotlinx.coroutines.CancellationException("cancelled").apply { initCause(e) }
        throw e
    } finally {
        runCatching { server.close() }
        watchdog.cancel()
    }
}

private const val COPY_BUFFER = 256 * 1024

/** Copies everything, reporting the running total at most every 64 KiB of progress. */
fun InputStream.copyWithProgress(out: java.io.OutputStream, onProgress: (Long) -> Unit): Long {
    val buffer = ByteArray(COPY_BUFFER)
    var total = 0L
    var reported = 0L
    while (true) {
        val n = read(buffer)
        if (n < 0) break
        out.write(buffer, 0, n)
        total += n
        if (total - reported >= 64 * 1024) {
            onProgress(total)
            reported = total
        }
    }
    onProgress(total)
    return total
}

/** Ends after [remaining] bytes and closes the socket with the stream. */
private class BoundedInputStream(input: InputStream, private var remaining: Long, private val owner: Closeable) : FilterInputStream(input) {
    override fun read(): Int {
        if (remaining <= 0) return -1
        val b = super.read()
        if (b >= 0) remaining--
        return b
    }

    override fun read(b: ByteArray, off: Int, len: Int): Int {
        if (remaining <= 0) return -1
        val n = super.read(b, off, minOf(len.toLong(), remaining).toInt())
        if (n > 0) remaining -= n
        if (n < 0 && remaining > 0) throw IOException("Payload ended $remaining bytes early")
        return n
    }

    override fun close() {
        runCatching { super.close() }
        owner.close()
    }
}
