package dev.vyoma.connect.protocol.plugin.builtin

import kotlin.concurrent.thread
import kotlinx.coroutines.launch
import kotlinx.coroutines.Dispatchers
import dev.vyoma.connect.protocol.NetworkPacket
import dev.vyoma.connect.protocol.PacketTypes
import dev.vyoma.connect.protocol.VLog
import dev.vyoma.connect.protocol.packet
import dev.vyoma.connect.protocol.plugin.Plugin
import dev.vyoma.connect.protocol.plugin.PluginContext
import dev.vyoma.connect.protocol.plugin.PluginSpec
import dev.vyoma.connect.protocol.putStrings
import kotlinx.serialization.json.addJsonObject
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonArray
import java.net.InetAddress
import java.security.PublicKey

/**
 * The phone's files in the PC's file manager, as KDE Connect's: the PC asks (`startBrowsing`), we
 * start an SFTP server and say where it is, with a one-time password; the PC mounts it with sshfs.
 * Our SSH host key is our TLS key, so a PC that pinned our certificate can check it's us; the PC
 * can sign in with its own certificate's key instead of the password.
 */
interface PhoneFileServer {
    class Running(val port: Int, val password: String, val roots: List<Pair<String, String>>)

    /**
     * Starts it (or keeps it running) for a PC whose certificate has [peerKey], which may connect
     * only [from] its links' addresses (loopback being the cable); why not, if it can't.
     */
    fun start(peerKey: PublicKey?, from: Collection<InetAddress>): Result<Running>

    /**
     * That PC is gone. Unpaired ([now]): its sessions end, and the server once no PC is left.
     * Only its link went: what it has open goes on (a mount outlives a blip, or the cable handing
     * over to Wi-Fi), and the server stops once idle.
     */
    fun stop(peerKey: PublicKey?, now: Boolean = true)

    /**
     * A small JPEG of the photo or video at [path] (as the server names it), at most [size] pixels
     * a side, for a PC that asked to browse ([peerKey]); null if it isn't one, or that PC may not.
     */
    fun thumbnail(peerKey: PublicKey?, path: String, size: Int): ByteArray? = null
}

class SftpPlugin(private val ctx: PluginContext, private val server: PhoneFileServer) : Plugin {
    /** Set once this PC has asked: whose share of the server to give back on stop. */
    @Volatile private var peerKey: PublicKey? = null
    override suspend fun onStart() {
        // As stock: the PC learns nothing is mounted yet.
        ctx.send(packet(PacketTypes.SFTP) { put("serverRunning", false) })
    }

    override suspend fun onPacket(packet: NetworkPacket): Boolean {
        if (packet.type == THUMBS_REQUEST) return thumbs(packet)
        if (packet.bool("startBrowsing") != true) return false
        val key = ctx.device.peerCertificate?.publicKey
        peerKey = key
        val addresses = ctx.device.addresses
        val at = ctx.device.ourAddress
        // Off the device's dispatcher: starting the server binds ports and can take a while, and
        // every other packet from this PC would wait behind it.
        ctx.scope.launch(Dispatchers.IO) {
            val reply = server.start(key, addresses).fold(
                onSuccess = { running ->
                    packet(PacketTypes.SFTP) {
                        // Over the cable alone there's no address to give: the PC reaches the port through adb.
                        put("ip", at?.hostAddress?.substringBefore('%') ?: "127.0.0.1")
                        put("port", running.port)
                        put("user", USER)
                        put("password", running.password)
                        put("path", running.roots.singleOrNull()?.first ?: "/")
                        putStrings("multiPaths", running.roots.map { it.first })
                        putStrings("pathNames", running.roots.map { it.second })
                    }
                },
                onFailure = { e ->
                    VLog.i(TAG, "not browsable: ${e.message}")
                    packet(PacketTypes.SFTP) { put("errorMessage", e.message ?: "Can't share this phone's files") }
                },
            )
            ctx.send(reply)
        }
        return true
    }

    /**
     * Vyoma's window (on the Mac and Windows) shows a folder's photos and videos small: Android's own
     * thumbnails, a few at a time, only for a PC that asked to browse on this link.
     */
    private fun thumbs(packet: NetworkPacket): Boolean {
        // The server knows which PCs asked to browse (it outlives a relink): it decides.
        val key = ctx.device.peerCertificate?.publicKey
        val n = packet.long("n") ?: 0
        val size = (packet.int("size") ?: 128).coerceIn(32, 256)
        val paths = packet.stringList("paths").take(MAX_THUMBS)
        ctx.scope.launch(Dispatchers.IO) {
            val made = paths.map { path -> path to if (key == null) null else runCatching { server.thumbnail(key, path, size) }.getOrNull() }
            ctx.send(
                packet(THUMBS) {
                    put("n", n)
                    putJsonArray("thumbs") {
                        made.forEach { (path, jpg) ->
                            addJsonObject {
                                put("path", path)
                                if (jpg != null) put("jpg", java.util.Base64.getEncoder().encodeToString(jpg))
                            }
                        }
                    }
                },
            )
        }
        return true
    }

    override fun onStop() {
        val key = peerKey ?: return
        val now = !ctx.device.state.value.isPaired
        // Stopping the server waits for it to close: not on the device's dispatcher.
        thread(name = "sftp-stop", isDaemon = true) { server.stop(key, now) }
    }

    companion object {
        const val ID = "sftp"
        const val USER = "kdeconnect"
        private const val TAG = "Sftp"
        /** PC → phone `{n, size, paths}` → `{n, thumbs: [{path, jpg?}]}` (Vyoma's, not stock). */
        const val THUMBS_REQUEST = "vyoma.files.thumbs.request"
        const val THUMBS = "vyoma.files.thumbs"
        private const val MAX_THUMBS = 32

        fun spec(server: PhoneFileServer) = PluginSpec(
            ID,
            incoming = setOf(PacketTypes.SFTP_REQUEST, THUMBS_REQUEST),
            outgoing = setOf(PacketTypes.SFTP, THUMBS),
        ) { SftpPlugin(it, server) }
    }
}
