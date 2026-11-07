package dev.vyoma.connect.protocol

import dev.vyoma.connect.protocol.crypto.LocalCredentials
import dev.vyoma.connect.protocol.device.FileTrustStore
import dev.vyoma.connect.protocol.device.TrustedDevice
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import java.io.File

class TrustStoreTest {
    @get:Rule val folder = TemporaryFolder()

    private fun device(id: String) = TrustedDevice(id, "pc", DeviceType.DESKTOP, LocalCredentials.generate(id).certificate)

    @Test fun aBrokenFileFallsBackToTheLastGoodOne() {
        val file = File(folder.root, "trust.json")
        val a = DeviceIdentity.newDeviceId()
        val b = DeviceIdentity.newDeviceId()
        FileTrustStore(file).apply { put(device(a)); put(device(b)) }
        file.writeText("{ cut off by a power fail")
        val store = FileTrustStore(file)
        assertNotNull(store.get(a)) // the backup is from before the last write
        assertNull(store.get(b))
    }

    @Test fun oneBadEntryDoesntCostTheOthers() {
        val file = File(folder.root, "trust.json")
        val good = DeviceIdentity.newDeviceId()
        FileTrustStore(file).put(device(good))
        file.writeText(file.readText().replaceFirst("{", """{"broken":{"name":"x","cert":"not base64!"},"""))
        val store = FileTrustStore(file)
        assertNotNull(store.get(good))
        assertEquals(1, store.all().size)
    }

    @Test fun anEmptyFileHasAPayloadIfItHasSomewhereToFetchFrom() {
        val packet = NetworkPacket("kdeconnect.share.request", kotlinx.serialization.json.buildJsonObject {})
        assertEquals(false, packet.hasPayload)
        assertEquals(true, packet.withPayload(0, kotlinx.serialization.json.buildJsonObject { put("port", kotlinx.serialization.json.JsonPrimitive(1739)) }).hasPayload)
    }
}
