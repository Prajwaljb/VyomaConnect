package dev.vyoma.connect.platform

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Environment
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
            subsystemFactories = listOf(SftpSubsystemFactory.Builder().build())
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

    private fun sha256(s: String) = MessageDigest.getInstance("SHA-256").digest(s.toByteArray())

    companion object {
        private const val TAG = "PhoneFiles"
        private val PORTS = 1739..1764

        init {
            // As KDE Connect's app: the platform's own crypto (no BouncyCastle registrar), NIO2
            // sockets, and no home folder lookup (Android has none to find).
            System.setProperty(SecurityUtils.SECURITY_PROVIDER_REGISTRARS, "")
            System.setProperty("org.apache.sshd.common.io.IoServiceFactoryFactory", "org.apache.sshd.common.io.nio2.Nio2ServiceFactoryFactory")
            PathUtils.setUserHomeFolderResolver { Path.of("/") }
        }
    }
}
