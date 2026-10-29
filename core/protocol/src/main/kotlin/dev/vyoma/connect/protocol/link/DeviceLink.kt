package dev.vyoma.connect.protocol.link

import dev.vyoma.connect.protocol.DeviceIdentity
import dev.vyoma.connect.protocol.NetworkPacket
import dev.vyoma.connect.protocol.ProtocolException
import dev.vyoma.connect.protocol.VLog
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import java.io.BufferedInputStream
import java.io.BufferedOutputStream
import java.io.ByteArrayOutputStream
import java.io.IOException
import java.io.InputStream
import java.net.InetAddress
import java.security.cert.X509Certificate
import java.util.concurrent.atomic.AtomicBoolean
import javax.net.ssl.SSLSocket

/** Declared in send-preference order: lowest latency first. */
enum class TransportKind { USB, LAN, BLUETOOTH }

/** An authenticated, encrypted connection to one peer. A device may have several (e.g. Wi-Fi + USB). */
class DeviceLink internal constructor(
    val identity: DeviceIdentity,
    val peerCertificate: X509Certificate,
    val transport: TransportKind,
    private val socket: SSLSocket,
    /** How binary payloads travel for this link; null if the transport can't carry them. */
    val payloads: PayloadTransport? = null,
) {
    private val input = BufferedInputStream(socket.inputStream, 64 * 1024)
    private val output = BufferedOutputStream(socket.outputStream, 64 * 1024)
    private val writeLock = Mutex()
    private val closed = AtomicBoolean(false)

    val remoteAddress: InetAddress get() = socket.inetAddress
    val isClosed: Boolean get() = closed.get()

    suspend fun send(packet: NetworkPacket): Boolean = withContext(Dispatchers.IO) {
        if (closed.get()) return@withContext false
        writeLock.withLock {
            try {
                output.write(packet.serialize().toByteArray(Charsets.UTF_8))
                output.flush()
                true
            } catch (e: IOException) {
                VLog.w(TAG, "send ${packet.type} to ${identity.deviceName} failed", e)
                close()
                false
            }
        }
    }

    /** Blocks (on IO) reading packets until the link drops. */
    suspend fun receive(onPacket: suspend (NetworkPacket) -> Unit) = withContext(Dispatchers.IO) {
        try {
            while (!closed.get()) {
                val line = input.readPacketLine() ?: break
                if (line.isBlank()) continue
                val packet = try {
                    NetworkPacket.parse(line)
                } catch (e: ProtocolException) {
                    VLog.w(TAG, "dropping malformed packet from ${identity.deviceName}", e)
                    continue
                }
                onPacket(packet)
            }
        } catch (e: IOException) {
            if (!closed.get()) VLog.d(TAG, "link to ${identity.deviceName} dropped: ${e.message}")
        } finally {
            close()
        }
    }

    fun close() {
        if (closed.compareAndSet(false, true)) runCatching { socket.close() }
    }

    override fun toString() = "DeviceLink(${identity.deviceName}, $transport, ${remoteAddress.hostAddress})"

    private companion object {
        const val TAG = "DeviceLink"
    }
}

/** Big enough for SMS conversation dumps, small enough to stop a hostile peer exhausting memory. */
internal const val MAX_PACKET_BYTES = 32 * 1024 * 1024

/**
 * Reads one '\n'-terminated line. Reads byte-by-byte from [this] so it never consumes
 * bytes past the newline, which matters on the plaintext socket right before TLS starts.
 */
internal fun InputStream.readPacketLine(limit: Int = MAX_PACKET_BYTES): String? {
    val buffer = ByteArrayOutputStream(512)
    while (true) {
        val b = read()
        if (b == -1) return if (buffer.size() == 0) null else buffer.toString(Charsets.UTF_8.name())
        if (b == '\n'.code) return buffer.toString(Charsets.UTF_8.name())
        if (buffer.size() >= limit) throw IOException("Packet exceeds $limit bytes")
        buffer.write(b)
    }
}
