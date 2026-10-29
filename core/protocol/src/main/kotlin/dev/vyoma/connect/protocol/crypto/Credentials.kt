package dev.vyoma.connect.protocol.crypto

import org.bouncycastle.asn1.x500.X500NameBuilder
import org.bouncycastle.asn1.x500.style.BCStyle
import org.bouncycastle.asn1.x500.style.IETFUtils
import org.bouncycastle.cert.jcajce.JcaX509CertificateConverter
import org.bouncycastle.cert.jcajce.JcaX509CertificateHolder
import org.bouncycastle.cert.jcajce.JcaX509v3CertificateBuilder
import org.bouncycastle.operator.jcajce.JcaContentSignerBuilder
import java.io.File
import java.math.BigInteger
import java.security.KeyPairGenerator
import java.security.KeyStore
import java.security.MessageDigest
import java.security.PrivateKey
import java.security.cert.CertificateFactory
import java.security.cert.X509Certificate
import java.time.Duration
import java.time.Instant
import java.util.Date

/** This device's identity key pair. The certificate CN must equal the device id (protocol v8). */
class LocalCredentials(
    val deviceId: String,
    val privateKey: PrivateKey,
    val certificate: X509Certificate,
) {
    companion object {
        /**
         * RSA-2048 self-signed cert, matching what KDE Connect generates. RSA keeps
         * compatibility with older GSConnect / KDE Connect peers.
         */
        fun generate(deviceId: String): LocalCredentials {
            val keyPair = KeyPairGenerator.getInstance("RSA").apply { initialize(2048) }.generateKeyPair()
            val name = X500NameBuilder(BCStyle.INSTANCE)
                .addRDN(BCStyle.CN, deviceId)
                .addRDN(BCStyle.O, "KDE")
                .addRDN(BCStyle.OU, "Kde connect")
                .build()
            val now = Instant.now()
            val holder = JcaX509v3CertificateBuilder(
                name,
                BigInteger.TEN,
                Date.from(now.minus(Duration.ofDays(365))),
                Date.from(now.plus(Duration.ofDays(365L * 10))),
                name,
                keyPair.public,
            ).build(JcaContentSignerBuilder("SHA256withRSA").build(keyPair.private))
            return LocalCredentials(deviceId, keyPair.private, JcaX509CertificateConverter().getCertificate(holder))
        }
    }
}

interface CredentialStore {
    fun load(): LocalCredentials?
    fun save(credentials: LocalCredentials)
}

/** PKCS#12 file store. The file lives in app-private storage; the password only satisfies the format. */
class FileCredentialStore(private val file: File) : CredentialStore {
    private val password = "vyoma".toCharArray()

    override fun load(): LocalCredentials? {
        if (!file.exists()) return null
        return runCatching {
            val ks = KeyStore.getInstance("PKCS12").apply { file.inputStream().use { load(it, password) } }
            val cert = ks.getCertificate(ALIAS) as X509Certificate
            val key = ks.getKey(ALIAS, password) as PrivateKey
            LocalCredentials(Certificates.commonName(cert) ?: return null, key, cert)
        }.getOrNull()
    }

    override fun save(credentials: LocalCredentials) {
        val ks = KeyStore.getInstance("PKCS12").apply { load(null, null) }
        ks.setKeyEntry(ALIAS, credentials.privateKey, password, arrayOf(credentials.certificate))
        file.parentFile?.mkdirs()
        val tmp = File(file.parentFile, file.name + ".tmp")
        tmp.outputStream().use { ks.store(it, password) }
        if (!tmp.renameTo(file)) {
            file.delete()
            tmp.renameTo(file)
        }
    }

    private companion object {
        const val ALIAS = "vyoma"
    }
}

object Certificates {
    fun commonName(cert: X509Certificate): String? = runCatching {
        JcaX509CertificateHolder(cert).subject.getRDNs(BCStyle.CN).firstOrNull()
            ?.first?.value?.let(IETFUtils::valueToString)
    }.getOrNull()

    fun encode(cert: X509Certificate): ByteArray = cert.encoded

    fun decode(der: ByteArray): X509Certificate =
        CertificateFactory.getInstance("X.509").generateCertificate(der.inputStream()) as X509Certificate

    fun sha256Fingerprint(cert: X509Certificate): String =
        MessageDigest.getInstance("SHA-256").digest(cert.encoded).joinToString(":") { "%02X".format(it) }

    /**
     * The 8-character code both sides show while pairing, as KDE Connect computes it:
     * SHA-256 over the two SubjectPublicKeyInfo blobs (larger first, unsigned byte order),
     * followed by the decimal pairing timestamp on protocol v8.
     */
    fun verificationKey(ours: X509Certificate, theirs: X509Certificate, pairingTimestamp: Long?): String {
        var a = ours.publicKey.encoded
        var b = theirs.publicKey.encoded
        if (compareUnsigned(a, b) < 0) a = b.also { b = a }
        val digest = MessageDigest.getInstance("SHA-256")
        digest.update(a)
        digest.update(b)
        if (pairingTimestamp != null) digest.update(pairingTimestamp.toString().toByteArray(Charsets.US_ASCII))
        return digest.digest().joinToString("") { "%02x".format(it) }.take(8).uppercase()
    }

    private fun compareUnsigned(a: ByteArray, b: ByteArray): Int {
        for (i in 0 until minOf(a.size, b.size)) {
            val cmp = (a[i].toInt() and 0xFF) - (b[i].toInt() and 0xFF)
            if (cmp != 0) return cmp
        }
        return a.size - b.size
    }
}
