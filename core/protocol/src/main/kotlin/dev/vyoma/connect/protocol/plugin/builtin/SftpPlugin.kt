package dev.vyoma.connect.protocol.plugin.builtin

import dev.vyoma.connect.protocol.NetworkPacket
import dev.vyoma.connect.protocol.PacketTypes
import dev.vyoma.connect.protocol.VLog
import dev.vyoma.connect.protocol.packet
import dev.vyoma.connect.protocol.plugin.Plugin
import dev.vyoma.connect.protocol.plugin.PluginContext
import dev.vyoma.connect.protocol.plugin.PluginSpec
import dev.vyoma.connect.protocol.putStrings
import kotlinx.serialization.json.put
import java.security.PublicKey

/**
 * The phone's files in the PC's file manager, as KDE Connect's: the PC asks (`startBrowsing`), we
 * start an SFTP server and say where it is, with a one-time password; the PC mounts it with sshfs.
 * Our SSH host key is our TLS key, so a PC that pinned our certificate can check it's us; the PC
 * can sign in with its own certificate's key instead of the password.
 */
interface PhoneFileServer {
    class Running(val port: Int, val password: String, val roots: List<Pair<String, String>>)

    /** Starts it (or keeps it running) for a PC whose certificate has [peerKey]; why not, if it can't. */
    fun start(peerKey: PublicKey?): Result<Running>

    /** That PC is gone: the server stops once no PC is left. */
    fun stop(peerKey: PublicKey?)
}

class SftpPlugin(private val ctx: PluginContext, private val server: PhoneFileServer) : Plugin {
    /** Set once this PC has asked: whose share of the server to give back on stop. */
    @Volatile private var peerKey: PublicKey? = null
    override suspend fun onStart() {
        // As stock: the PC learns nothing is mounted yet.
        ctx.send(packet(PacketTypes.SFTP) { put("serverRunning", false) })
    }

    override suspend fun onPacket(packet: NetworkPacket): Boolean {
        if (packet.bool("startBrowsing") != true) return false
        peerKey = ctx.device.peerCertificate?.publicKey
        val reply = server.start(peerKey).fold(
            onSuccess = { running ->
                val at = ctx.device.ourAddress
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
        return true
    }

    override fun onStop() {
        if (peerKey != null) server.stop(peerKey)
    }

    companion object {
        const val ID = "sftp"
        const val USER = "kdeconnect"
        private const val TAG = "Sftp"

        fun spec(server: PhoneFileServer) = PluginSpec(
            ID,
            incoming = setOf(PacketTypes.SFTP_REQUEST),
            outgoing = setOf(PacketTypes.SFTP),
        ) { SftpPlugin(it, server) }
    }
}
