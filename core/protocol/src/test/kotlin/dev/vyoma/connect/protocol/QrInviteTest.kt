package dev.vyoma.connect.protocol

import dev.vyoma.connect.protocol.crypto.QrInvite
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertNull

class QrInviteTest {
    private val code = "https://github.com/Prajwaljb/VyomaConnect/releases/latest#pair?v=1&id=0123456789abcdef0123456789abcdef" +
        "&name=Desk%20PC&fp=1b9f3d973c838fe866cb806b72ef20448ccb55c320a514b35e9e03808d00069b" +
        "&s=00112233445566778899aabbccddeeff&a=192.168.1.20,10.0.0.5&p=1717"

    @Test
    fun readsThePcsCode() {
        val invite = assertNotNull(QrInvite.parse(code))
        assertEquals("0123456789abcdef0123456789abcdef", invite.deviceId)
        assertEquals("Desk PC", invite.name)
        assertEquals(listOf("192.168.1.20", "10.0.0.5"), invite.addresses)
        assertEquals(1717, invite.port)
        assertEquals(16, invite.secret.size)
        assertEquals(0x11.toByte(), invite.secret[1])
        assertNotNull(QrInvite.parse(code.replace("https://github.com/Prajwaljb/VyomaConnect/releases/latest#", "vyoma://")))
    }

    @Test
    fun refusesWhatIsntOne() {
        assertNull(QrInvite.parse("https://example.com/#pair?v=1&id=x"))
        assertNull(QrInvite.parse(code.replace("v=1", "v=2")))
        assertNull(QrInvite.parse(code.replace("&s=00112233445566778899aabbccddeeff", "&s=short")))
        assertNull(QrInvite.parse(code.replace("fp=1b9f", "fp=zz9f")))
        // Only IP literals: a name would need a lookup someone could answer falsely.
        assertNull(QrInvite.parse(code.replace("a=192.168.1.20,10.0.0.5", "a=evil.example")))
        assertEquals(listOf("10.0.0.5"), QrInvite.parse(code.replace("a=192.168.1.20,", "a=999.1.1.1,"))?.addresses)
    }

    @Test
    fun makesTheSameProofAsThePc() {
        // desktop/core/src/invite.rs matches_the_phone, and Python's hmac, give this.
        assertEquals(
            "1b9f3d973c838fe866cb806b72ef20448ccb55c320a514b35e9e03808d00069b",
            QrInvite.proof(ByteArray(16) { 7 }, 1_700_000_000, "phone".toByteArray(), "pc".toByteArray()),
        )
    }
}
