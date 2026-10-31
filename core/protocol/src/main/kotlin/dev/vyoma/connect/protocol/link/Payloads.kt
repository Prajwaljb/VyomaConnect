package dev.vyoma.connect.protocol.link

import dev.vyoma.connect.protocol.NetworkPacket
import dev.vyoma.connect.protocol.ProtocolException
import dev.vyoma.connect.protocol.crypto.TlsContext
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.launch
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
    /** Announces [packet] with [source] attached and streams it. Returns true once fully sent. */
    suspend fun send(link: DeviceLink, packet: NetworkPacket, source: PayloadSource, timeoutMs: Int, onProgress: (Long) -> Unit): Boolean

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

    override suspend fun send(
        link: DeviceLink,
        packet: NetworkPacket,
        source: PayloadSource,
        timeoutMs: Int,
        onProgress: (Long) -> Unit,
    ): Boolean = withContext(Dispatchers.IO) {
        val server = bindPayloadPort()
        var ssl: SSLSocket? = null
        closeOnCancel(server, { ssl }) {
            server.soTimeout = timeoutMs
            val announced = packet.withPayload(source.size, buildJsonObject { put("port", server.localPort) })
            if (!link.send(announced)) return@closeOnCancel false
            val plain = server.accept()
            tuneSocket(plain)
            val secure = tls.wrap(plain, asServer = true).also { ssl = it }
            secure.use {
                it.soTimeout = IO_TIMEOUT_MS
                it.startHandshake()
                verifyPeer(it, link.peerCertificate)
                source.open().use { input -> input.copyWithProgress(it.outputStream, onProgress) }
                it.outputStream.flush()
            }
            true
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
        try {
            route(plain, host)
            plain.connect(InetSocketAddress(host, target), CONNECT_TIMEOUT_MS)
            tuneSocket(plain)
            val ssl = tls.wrap(plain, asServer = false)
            ssl.soTimeout = IO_TIMEOUT_MS
            ssl.startHandshake()
            verifyPeer(ssl, link.peerCertificate)
            // An open-ended stream (the PC's live screen) goes quiet whenever nothing changes: no read timeout.
            if (packet.payloadSize < 0) ssl.also { it.soTimeout = 0 }.inputStream else BoundedInputStream(ssl.inputStream, packet.payloadSize, ssl)
        } catch (e: Exception) {
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
            try {
                return ServerSocket().apply {
                    reuseAddress = true
                    bind(InetSocketAddress(port))
                }
            } catch (_: IOException) {
                // in use (another transfer), try the next one
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
        const val CONNECT_TIMEOUT_MS = 5_000
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
