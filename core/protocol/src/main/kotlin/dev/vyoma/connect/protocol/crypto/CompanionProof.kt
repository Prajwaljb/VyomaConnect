package dev.vyoma.connect.protocol.crypto

import java.security.MessageDigest
import java.security.Signature
import java.security.cert.X509Certificate
import java.util.Base64
import kotlin.math.abs

/**
 * Lets the Vyoma companion on a PC pair without a prompt. The companion signs this message
 * with the key of the KDE Connect daemon on the same PC, which we already trust; it binds
 * the companion's own certificate and this phone, so a captured proof can't be reused.
 * Mirrors `proof_message` in desktop/companion/src/kde.rs.
 */
object CompanionProof {
    private const val MAX_SKEW_S = 600L

    fun message(companionId: String, companionCert: X509Certificate, phoneId: String, timestamp: Long): String {
        val fingerprint = MessageDigest.getInstance("SHA-256").digest(companionCert.encoded).joinToString("") { "%02x".format(it) }
        return "vyoma-companion-v1\n$companionId\n$fingerprint\n$phoneId\n$timestamp"
    }

    fun verify(
        hostCert: X509Certificate,
        companionId: String,
        companionCert: X509Certificate,
        phoneId: String,
        timestamp: Long,
        proof: String,
        nowSeconds: Long = System.currentTimeMillis() / 1000,
    ): Boolean {
        if (abs(nowSeconds - timestamp) > MAX_SKEW_S) return false
        val algorithm = when (hostCert.publicKey.algorithm) {
            "EC" -> "SHA256withECDSA"
            "RSA" -> "SHA256withRSA"
            else -> return false
        }
        return runCatching {
            Signature.getInstance(algorithm).run {
                initVerify(hostCert.publicKey)
                update(message(companionId, companionCert, phoneId, timestamp).toByteArray(Charsets.UTF_8))
                verify(Base64.getDecoder().decode(proof))
            }
        }.getOrDefault(false)
    }
}
