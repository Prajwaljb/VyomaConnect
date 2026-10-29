package dev.vyoma.connect.protocol.crypto

import java.net.Socket
import java.security.Principal
import java.security.PrivateKey
import java.security.SecureRandom
import java.security.cert.X509Certificate
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLEngine
import javax.net.ssl.SSLSocket
import javax.net.ssl.X509ExtendedKeyManager
import javax.net.ssl.X509ExtendedTrustManager

/**
 * TLS for KDE Connect links. Every peer uses a self-signed certificate, so the TLS layer
 * accepts any certificate and trust is decided afterwards by pinning: the
 * certificate stored at pairing time must match exactly (see DeviceHub).
 */
class TlsContext(private val credentials: LocalCredentials) {
    private val context: SSLContext = SSLContext.getInstance("TLS").apply {
        init(arrayOf(FixedKeyManager(credentials)), arrayOf(AcceptAllTrustManager), SecureRandom())
    }

    private val protocols: Array<String> by lazy {
        val supported = context.supportedSSLParameters.protocols.toSet()
        arrayOf("TLSv1.3", "TLSv1.2").filter { it in supported }.toTypedArray()
    }

    val certificate: X509Certificate get() = credentials.certificate

    /**
     * Upgrades an established TCP socket. In KDE Connect the side that opened the TCP
     * connection acts as the TLS *server*, so pass `asServer = true` for outgoing sockets.
     */
    fun wrap(plain: Socket, asServer: Boolean): SSLSocket {
        val ssl = context.socketFactory.createSocket(
            plain, plain.inetAddress.hostAddress, plain.port, true,
        ) as SSLSocket
        ssl.enabledProtocols = protocols
        ssl.useClientMode = !asServer
        if (asServer) ssl.needClientAuth = true
        return ssl
    }

    private class FixedKeyManager(private val creds: LocalCredentials) : X509ExtendedKeyManager() {
        private val alias = "vyoma"
        override fun chooseClientAlias(keyType: Array<out String>?, issuers: Array<out Principal>?, socket: Socket?) = alias
        override fun chooseServerAlias(keyType: String?, issuers: Array<out Principal>?, socket: Socket?) = alias
        override fun chooseEngineClientAlias(keyType: Array<out String>?, issuers: Array<out Principal>?, engine: SSLEngine?) = alias
        override fun chooseEngineServerAlias(keyType: String?, issuers: Array<out Principal>?, engine: SSLEngine?) = alias
        override fun getClientAliases(keyType: String?, issuers: Array<out Principal>?) = arrayOf(alias)
        override fun getServerAliases(keyType: String?, issuers: Array<out Principal>?) = arrayOf(alias)
        override fun getCertificateChain(alias: String?): Array<X509Certificate> = arrayOf(creds.certificate)
        override fun getPrivateKey(alias: String?): PrivateKey = creds.privateKey
    }

    private object AcceptAllTrustManager : X509ExtendedTrustManager() {
        override fun checkClientTrusted(chain: Array<out X509Certificate>?, authType: String?) = Unit
        override fun checkServerTrusted(chain: Array<out X509Certificate>?, authType: String?) = Unit
        override fun checkClientTrusted(chain: Array<out X509Certificate>?, authType: String?, socket: Socket?) = Unit
        override fun checkServerTrusted(chain: Array<out X509Certificate>?, authType: String?, socket: Socket?) = Unit
        override fun checkClientTrusted(chain: Array<out X509Certificate>?, authType: String?, engine: SSLEngine?) = Unit
        override fun checkServerTrusted(chain: Array<out X509Certificate>?, authType: String?, engine: SSLEngine?) = Unit
        override fun getAcceptedIssuers(): Array<X509Certificate> = emptyArray()
    }
}
