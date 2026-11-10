package dev.vyoma.connect.protocol.crypto

import java.net.URLDecoder
import java.security.MessageDigest
import java.security.cert.X509Certificate
import javax.crypto.Mac
import javax.crypto.spec.SecretKeySpec

/**
 * Pairing by QR code, the phone's half (desktop/core/src/invite.rs is the PC's). Vyoma's PC window
 * shows a code with the PC's id, name, certificate fingerprint, addresses and port, and a one-time
 * secret. Scanned, the phone dials those addresses, pairs only with a PC showing that certificate,
 * and sends its pair request with [proof]: the PC pairs on it with no code to compare.
 *
 * The code is the release page's link with all of that after the `#`, so a camera app without Vyoma
 * opens the download instead; browsers never send what follows `#`. `vyoma://pair?...` works too.
 */
class QrInvite(
    val deviceId: String,
    val name: String,
    /** SHA-256 of the PC's certificate, lowercase hex. */
    val fingerprint: String,
    val secret: ByteArray,
    /** IP literals only, best first: nothing to look up, so nothing to spoof. */
    val addresses: List<String>,
    val port: Int,
) {
    /** Whether [cert] is the one the code was made for. */
    fun matches(cert: X509Certificate): Boolean =
        MessageDigest.isEqual(fingerprint(cert.encoded).toByteArray(), fingerprint.toByteArray())

    companion object {
        /** The pair request's field that carries the proof. */
        const val FIELD = "vyomaInvite"
        private val PORTS = 1716..1764
        private val HEX64 = Regex("[0-9a-f]{64}")
        private val HEX32 = Regex("[0-9a-f]{32}")
        private val ID = Regex("[A-Za-z0-9_-]{16,64}")
        private val IPV4 = Regex("""(25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)(\.(25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)){3}""")
        private val IPV6 = Regex("[0-9A-Fa-f:]{2,39}")

        /** The invite in a scanned code, or null if it isn't one of Vyoma's. */
        fun parse(text: String): QrInvite? {
            val at = text.indexOf("pair?")
            if (at < 0 || !(text.startsWith("vyoma://") || text.startsWith("https://github.com/"))) return null
            val params = text.substring(at + 5).split('&').mapNotNull { part ->
                val eq = part.indexOf('=')
                if (eq <= 0) null else part.substring(0, eq) to runCatching { URLDecoder.decode(part.substring(eq + 1), "UTF-8") }.getOrNull()
            }.toMap()
            if (params["v"] != "1") return null
            val id = params["id"]?.takeIf { ID.matches(it) } ?: return null
            val fp = params["fp"]?.takeIf { HEX64.matches(it) } ?: return null
            val secret = params["s"]?.takeIf { HEX32.matches(it) }?.chunked(2)?.map { it.toInt(16).toByte() }?.toByteArray() ?: return null
            val addresses = params["a"].orEmpty().split(',').map { it.trim() }
                .filter { (IPV4.matches(it) || (it.contains(':') && IPV6.matches(it))) && dialable(it) }.take(4)
            if (addresses.isEmpty()) return null
            // KDE Connect's own ports only: a code can't point the phone at some other service.
            val port = params["p"]?.toIntOrNull()?.takeIf { it in PORTS } ?: return null
            // Shown on screen: no control or invisible format characters (a right-to-left override could
            // make a name read as something else).
            val name = params["name"].orEmpty().filter { !it.isISOControl() && Character.getType(it) != Character.FORMAT.toInt() }
                .take(64).trim().ifBlank { "your PC" }
            return QrInvite(id, name, fp, secret, addresses, port)
        }

        /** A PC's address: not this phone, nor every device on the network at once. */
        private fun dialable(literal: String): Boolean {
            // A literal: getByName parses it without looking anything up.
            val ip = runCatching { java.net.InetAddress.getByName(literal) }.getOrNull() ?: return false
            return !(ip.isLoopbackAddress || ip.isAnyLocalAddress || ip.isMulticastAddress || literal == "255.255.255.255")
        }

        fun fingerprint(der: ByteArray): String = hex(MessageDigest.getInstance("SHA-256").digest(der))

        /**
         * HMAC-SHA256 with the code's secret over "vyoma-invite-v1|<timestamp>|<phone cert SHA-256>|<PC cert SHA-256>"
         * (fingerprints in lowercase hex), as lowercase hex. Both certificates are in it, so it only
         * works on the link it was made for.
         */
        fun proof(secret: ByteArray, timestamp: Long, phoneCert: ByteArray, pcCert: ByteArray): String {
            val mac = Mac.getInstance("HmacSHA256")
            mac.init(SecretKeySpec(secret, "HmacSHA256"))
            return hex(mac.doFinal("vyoma-invite-v1|$timestamp|${fingerprint(phoneCert)}|${fingerprint(pcCert)}".toByteArray(Charsets.UTF_8)))
        }

        private fun hex(bytes: ByteArray) = bytes.joinToString("") { "%02x".format(it) }
    }
}
