package dev.vyoma.connect.protocol

import dev.vyoma.connect.protocol.crypto.Certificates
import dev.vyoma.connect.protocol.crypto.LocalCredentials
import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertTrue
import kotlinx.serialization.json.put

class ProtocolTest {
    @Test
    fun packetRoundTrip() {
        val original = packet(PacketTypes.PING) { put("message", "hi") }
        val parsed = NetworkPacket.parse(original.serialize().trimEnd())
        assertEquals(original.type, parsed.type)
        assertEquals(original.id, parsed.id)
        assertEquals("hi", parsed.string("message"))
    }

    @Test
    fun lenientNumbers() {
        val p = NetworkPacket.parse("""{"id":"123","type":"x","body":{"n":"42","b":"true"}}""")
        assertEquals(123L, p.id)
        assertEquals(42, p.int("n"))
        assertEquals(true, p.bool("b"))
    }

    @Test
    fun rejectsGarbage() {
        assertFailsWith<ProtocolException> { NetworkPacket.parse("not json") }
        assertFailsWith<ProtocolException> { NetworkPacket.parse("""{"id":1,"body":{}}""") }
    }

    @Test
    fun identityRoundTripAndSanitizing() {
        val id = DeviceIdentity(
            DeviceIdentity.newDeviceId(), DeviceIdentity.sanitizeName("Raz's (S24) FE!"), DeviceType.PHONE,
            8, setOf("kdeconnect.ping"), setOf("kdeconnect.ping"), 1716,
        )
        assertEquals("Razs S24 FE", id.deviceName)
        assertEquals(id, DeviceIdentity.fromPacket(NetworkPacket.parse(id.toPacket().serialize())))
    }

    @Test
    fun certificateAndVerificationKey() {
        val a = LocalCredentials.generate(DeviceIdentity.newDeviceId())
        val b = LocalCredentials.generate(DeviceIdentity.newDeviceId())
        assertEquals(a.deviceId, Certificates.commonName(a.certificate))
        val ab = Certificates.verificationKey(a.certificate, b.certificate, 1_700_000_000)
        val ba = Certificates.verificationKey(b.certificate, a.certificate, 1_700_000_000)
        assertEquals(ab, ba, "both sides must compute the same code")
        assertTrue(ab.matches(Regex("[0-9A-F]{8}")))
    }

    @Test
    fun linkedVCardsMergeIntoOne() {
        val raw = "BEGIN:VCARD\r\nVERSION:2.1\r\nN:Doe;Jane;;;\r\nFN:Jane Doe\r\nTEL;CELL:+91 98765 43210\r\nEND:VCARD\r\n" +
            "BEGIN:VCARD\r\nVERSION:2.1\r\nN:Doe;Jane;;;\r\nFN:Jane D\r\nTEL;TYPE=CELL:09876543210\r\nEMAIL:Jane@Example.com\r\nEND:VCARD\r\n"
        val merged = dev.vyoma.connect.protocol.plugin.builtin.ContactsPlugin.mergeVCards(raw).lines()
        assertEquals(1, merged.count { it == "BEGIN:VCARD" })
        assertEquals(1, merged.count { it.startsWith("FN:") })
        assertEquals(1, merged.count { it.startsWith("TEL") }, "same number in two formats counts once")
        assertTrue("EMAIL:Jane@Example.com" in merged)
        assertEquals("END:VCARD", merged.last())
    }
}
