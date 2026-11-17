package dev.vyoma.connect.protocol

import dev.vyoma.connect.protocol.crypto.LocalCredentials
import dev.vyoma.connect.protocol.device.DeviceHub
import dev.vyoma.connect.protocol.device.TrustStore
import dev.vyoma.connect.protocol.device.TrustedDevice
import dev.vyoma.connect.protocol.link.LanBackend
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** A PC that said it's going away isn't looked for (no retries spending the phone's battery) until it's back or asked for. */
class RestingTest {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val pc = LocalCredentials.generate(DeviceIdentity.newDeviceId())
    private val trust = object : TrustStore {
        val map = mutableMapOf(pc.deviceId to TrustedDevice(pc.deviceId, "PC", DeviceType.DESKTOP, pc.certificate, lastAddress = "192.0.2.1"))
        override fun all() = map.values.toList()
        override fun get(deviceId: String) = map[deviceId]
        override fun put(device: TrustedDevice) { map[device.deviceId] = device }
        override fun remove(deviceId: String) { map.remove(deviceId) }
    }
    private val hub = DeviceHub(
        credentials = LocalCredentials.generate(DeviceIdentity.newDeviceId()),
        trustStore = trust,
        pluginSpecs = emptyList(),
        scope = scope,
        deviceName = "phone",
        deviceType = DeviceType.PHONE,
        lanConfig = LanBackend.Config(listenUdp = false, broadcastTargets = { emptyList() }),
    )

    @After fun tearDown() = scope.cancel()

    @Test fun aPcThatSaidItsLeavingIsntLookedForUntilAskedAgain() {
        assertTrue(hub.anyPairedOffline())
        hub.onLeaving(pc.deviceId, "shutdown")
        assertEquals(mapOf(pc.deviceId to "shutdown"), hub.resting.value)
        assertFalse("nothing left worth retrying", hub.anyPairedOffline())
        hub.stopResting()
        assertTrue(hub.resting.value.isEmpty())
        assertTrue(hub.anyPairedOffline())
    }
}
