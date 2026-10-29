package dev.vyoma.connect.protocol

import dev.vyoma.connect.protocol.crypto.CompanionProof
import dev.vyoma.connect.protocol.crypto.LocalCredentials
import org.bouncycastle.asn1.x500.X500Name
import org.bouncycastle.cert.jcajce.JcaX509CertificateConverter
import org.bouncycastle.cert.jcajce.JcaX509v3CertificateBuilder
import org.bouncycastle.operator.jcajce.JcaContentSignerBuilder
import org.junit.Test
import java.math.BigInteger
import java.security.KeyPair
import java.security.KeyPairGenerator
import java.security.Signature
import java.security.cert.X509Certificate
import java.security.spec.ECGenParameterSpec
import java.util.Base64
import java.util.Date
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class CompanionProofTest {
    // The PC's KDE Connect key: EC P-256, like kdeconnectd 24+ generates.
    private val hostKeys: KeyPair = KeyPairGenerator.getInstance("EC").apply { initialize(ECGenParameterSpec("secp256r1")) }.generateKeyPair()
    private val hostCert: X509Certificate = selfSigned(hostKeys, "529a9b17e4fd4c3e8628168394abbc4b")
    private val companion = LocalCredentials.generate("companion_0000_0000_0000_000000000000")
    private val phoneId = "2f728e44_c76d_4b97_a98d_d2f055624b0c"
    private val now = 1_800_000_000L

    private fun sign(message: String, keys: KeyPair = hostKeys): String = Signature.getInstance("SHA256withECDSA").run {
        initSign(keys.private)
        update(message.toByteArray())
        Base64.getEncoder().encodeToString(sign())
    }

    private fun proof(ts: Long = now, phone: String = phoneId, keys: KeyPair = hostKeys) =
        sign(CompanionProof.message(companion.deviceId, companion.certificate, phone, ts), keys)

    private fun verify(proof: String, ts: Long = now) =
        CompanionProof.verify(hostCert, companion.deviceId, companion.certificate, phoneId, ts, proof, nowSeconds = now)

    @Test fun acceptsValidProof() = assertTrue(verify(proof()))

    @Test fun rejectsProofForAnotherPhone() = assertFalse(verify(proof(phone = "another_phone_id_000000000000000")))

    @Test fun rejectsStaleProof() = assertFalse(verify(proof(ts = now - 3600), ts = now - 3600))

    @Test fun rejectsOtherKey() {
        val stranger = KeyPairGenerator.getInstance("EC").apply { initialize(ECGenParameterSpec("secp256r1")) }.generateKeyPair()
        assertFalse(verify(proof(keys = stranger)))
    }

    @Test fun rejectsGarbage() = assertFalse(verify("not base64!"))

    private fun selfSigned(keys: KeyPair, cn: String): X509Certificate {
        val name = X500Name("CN=$cn,O=KDE,OU=KDE Connect")
        val holder = JcaX509v3CertificateBuilder(name, BigInteger.ONE, Date(0), Date(4_000_000_000_000), name, keys.public)
            .build(JcaContentSignerBuilder("SHA256withECDSA").build(keys.private))
        return JcaX509CertificateConverter().getCertificate(holder)
    }
}
