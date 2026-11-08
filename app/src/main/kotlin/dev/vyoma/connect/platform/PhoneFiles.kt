package dev.vyoma.connect.platform

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Environment
import android.os.StatFs
import android.provider.Settings
import dev.vyoma.connect.protocol.VLog
import dev.vyoma.connect.protocol.plugin.builtin.PhoneFileServer
import dev.vyoma.connect.protocol.plugin.builtin.SftpPlugin
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import org.apache.sshd.common.file.virtualfs.VirtualFileSystemFactory
import org.apache.sshd.common.keyprovider.KeyPairProvider
import org.apache.sshd.common.util.io.PathUtils
import org.apache.sshd.common.util.security.SecurityUtils
import org.apache.sshd.server.SshServer
import org.apache.sshd.server.auth.password.PasswordAuthenticator
import org.apache.sshd.server.auth.pubkey.PublickeyAuthenticator
import org.apache.sshd.server.channel.ChannelSession
import org.apache.sshd.server.session.ServerSession
import org.apache.sshd.sftp.common.SftpConstants
import org.apache.sshd.sftp.common.extensions.openssh.AbstractOpenSSHExtensionParser.OpenSSHExtension
import org.apache.sshd.common.util.buffer.Buffer
import org.apache.sshd.sftp.server.SftpSubsystem
import org.apache.sshd.sftp.server.SftpSubsystemConfigurator
import org.apache.sshd.sftp.server.SftpSubsystemFactory
import java.io.IOException
import java.nio.file.Path
import java.security.KeyPair
import java.security.MessageDigest
import java.security.PublicKey
import java.security.SecureRandom

/**
 * "Browse this phone from your PC": off until turned on in Settings, and then only with Android's
 * All files access, which the switch asks for.
 */
object PhoneFileSharing {
    private const val PREFS = "phone_files"
    private const val ON = "on"
    private val _on = MutableStateFlow(false)
    val on: StateFlow<Boolean> = _on.asStateFlow()

    fun init(context: Context) {
        _on.value = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getBoolean(ON, false)
    }

    fun setOn(context: Context, on: Boolean) {
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putBoolean(ON, on).apply()
        _on.value = on
    }

    fun allowed() = Environment.isExternalStorageManager()

    fun accessIntent(context: Context) =
        Intent(Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION, Uri.parse("package:${context.packageName}"))
}

/**
 * The SFTP server the PC mounts (Apache MINA SSHD, as KDE Connect's app), rooted at internal storage.
 * Its host key is our TLS key, so the PC can check it against the certificate it pinned; a PC signs
 * in with its own certificate's key, or the password we sent it over the TLS link.
 */
class AndroidPhoneFileServer(private val app: Context, private val hostKey: () -> KeyPair) : PhoneFileServer {
    private val lock = Any()
    private var sshd: SshServer? = null
    /** The PCs that asked, while they're here: who may sign in by key, and who keeps it running. */
    private val peers = mutableSetOf<PublicKey>()
    @Volatile private var passwordHash = ByteArray(0)

    init {
        PhoneFileSharing.init(app)
    }

    override fun start(peerKey: PublicKey?): Result<PhoneFileServer.Running> = runCatching {
        synchronized(lock) {
            check(PhoneFileSharing.on.value) { "Turn on \"Browse this phone from your PC\" in Vyoma's settings" }
            check(PhoneFileSharing.allowed()) { "Vyoma needs All files access on the phone to share its files" }
            peerKey?.let(peers::add)
            val server = sshd?.takeIf { it.isStarted } ?: open().also { sshd = it }
            // A fresh one each time it's asked for: sessions already in keep going.
            val password = ByteArray(21).also(SecureRandom()::nextBytes).let { android.util.Base64.encodeToString(it, android.util.Base64.NO_WRAP or android.util.Base64.URL_SAFE) }
            passwordHash = sha256(password)
            VLog.i(TAG, "browsable on port ${server.port}")
            PhoneFileServer.Running(server.port, password, listOf("/" to "Internal storage"))
        }
    }

    override fun stop(peerKey: PublicKey?) = synchronized(lock) {
        peerKey?.let(peers::remove)
        if (peers.isEmpty()) close()
    }

    /** Turned off in Settings: whoever's in is cut off now. */
    fun stopAll() = synchronized(lock) {
        peers.clear()
        close()
    }

    private fun close() {
        sshd?.let { s ->
            runCatching { s.stop(true) }
            VLog.i(TAG, "stopped")
        }
        sshd = null
    }

    private fun open(): SshServer {
        val server = SshServer.setUpDefaultServer().apply {
            keyPairProvider = KeyPairProvider.wrap(hostKey())
            fileSystemFactory = VirtualFileSystemFactory(Environment.getExternalStorageDirectory().toPath())
            subsystemFactories = listOf(FreeSpaceSftpFactory())
            passwordAuthenticator = PasswordAuthenticator { user, password, _ ->
                user == SftpPlugin.USER && MessageDigest.isEqual(sha256(password), passwordHash)
            }
            publickeyAuthenticator = PublickeyAuthenticator { user, key, _ ->
                user == SftpPlugin.USER && synchronized(lock) { peers.any { it.encoded.contentEquals(key.encoded) } }
            }
        }
        // In KDE Connect's range, which the PC's adb forward also covers, so it works over the cable.
        for (port in PORTS) {
            try {
                server.port = port
                server.start()
                return server
            } catch (_: IOException) {
                // Taken (a payload, likely): the next one.
            }
        }
        error("No free port for the file server")
    }

    /**
     * MINA's SFTP, plus OpenSSH's free-space query (which it lacks), so the PC sees the phone's real space.
     * Kept in its package by proguard-rules.pro: MINA names its vendor from the subsystem's package.
     */
    private class FreeSpaceSftpFactory : SftpSubsystemFactory() {
        override fun createSubsystem(channel: ChannelSession) = FreeSpaceSftp(channel, this)
    }

    private class FreeSpaceSftp(channel: ChannelSession, config: SftpSubsystemConfigurator) : SftpSubsystem(channel, config) {
        override fun resolveOpenSSHExtensions(session: ServerSession): List<OpenSSHExtension> =
            super.resolveOpenSSHExtensions(session) + OpenSSHExtension(STATVFS, "2")

        override fun executeExtendedCommand(buffer: Buffer, id: Int, extension: String) {
            if (extension != STATVFS) return super.executeExtendedCommand(buffer, id, extension)
            buffer.getString() // The path: everything we serve is internal storage, one volume.
            val fs = StatFs(Environment.getExternalStorageDirectory().path)
            val reply = prepareReply(buffer)
            reply.putByte(SftpConstants.SSH_FXP_EXTENDED_REPLY.toByte())
            reply.putInt(id.toLong())
            // struct statvfs as OpenSSH sends it: bsize, frsize, blocks, bfree, bavail, files, ffree, favail, fsid, flag, namemax.
            val block = fs.blockSizeLong
            for (v in longArrayOf(block, block, fs.blockCountLong, fs.freeBlocksLong, fs.availableBlocksLong, 0, 0, 0, 0, ST_NOSUID, 255)) reply.putLong(v)
            send(reply)
        }
    }

    private fun sha256(s: String) = MessageDigest.getInstance("SHA-256").digest(s.toByteArray())

    companion object {
        private const val TAG = "PhoneFiles"
        private val PORTS = 1739..1764
        private const val STATVFS = "statvfs@openssh.com"
        private const val ST_NOSUID = 2L

        init {
            // As KDE Connect's app: the platform's own crypto (no BouncyCastle registrar), NIO2
            // sockets, and no home folder lookup (Android has none to find).
            System.setProperty(SecurityUtils.SECURITY_PROVIDER_REGISTRARS, "")
            System.setProperty("org.apache.sshd.common.io.IoServiceFactoryFactory", "org.apache.sshd.common.io.nio2.Nio2ServiceFactoryFactory")
            PathUtils.setUserHomeFolderResolver { Path.of("/") }
        }
    }
}
