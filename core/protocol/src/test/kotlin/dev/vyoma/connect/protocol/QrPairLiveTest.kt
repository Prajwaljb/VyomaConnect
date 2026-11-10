package dev.vyoma.connect.protocol

import dev.vyoma.connect.protocol.crypto.LocalCredentials
import dev.vyoma.connect.protocol.crypto.QrInvite
import dev.vyoma.connect.protocol.device.DeviceHub
import dev.vyoma.connect.protocol.device.FileTrustStore
import dev.vyoma.connect.protocol.device.PairState
import dev.vyoma.connect.protocol.link.LanBackend
import dev.vyoma.connect.protocol.plugin.builtin.PingPlugin
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assume.assumeTrue
import org.junit.Test
import java.net.InetAddress
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertTrue

/**
 * Pairs a throwaway identity with this PC's running vyoma-desktop by the code its window shows,
 * then unpairs. The code comes in by environment so it's never printed:
 *   VYOMA_QR_CODE="$(busctl --user call dev.vyoma.Desktop /dev/vyoma/Desktop dev.vyoma.Desktop1 PairingCode | cut -d'"' -f2)" \
 *     ./gradlew :core:protocol:test --tests '*QrPairLiveTest*' --rerun
 */
class QrPairLiveTest {
    @Test
    fun pairsByTheCodeWithNoPrompt() = runBlocking {
        val code = System.getenv("VYOMA_QR_CODE")
        assumeTrue("no code given", !code.isNullOrBlank())
        val invite = assertNotNull(QrInvite.parse(code!!), "not a Vyoma code")
        val dir = kotlin.io.path.createTempDirectory("vyoma-qr").toFile()
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
        val hub = DeviceHub(
            credentials = LocalCredentials.generate(DeviceIdentity.newDeviceId()),
            trustStore = FileTrustStore(java.io.File(dir, "trusted.json")),
            pluginSpecs = listOf(PingPlugin.spec),
            scope = scope,
            deviceName = "Vyoma QR Test",
            deviceType = DeviceType.PHONE,
            lanConfig = LanBackend.Config(listenUdp = false, tcpPorts = 1717..1764, broadcastTargets = { emptyList() }),
        )
        try {
            hub.dial(InetAddress.getByName(invite.addresses.first()), invite.port)
            val device = withTimeout(10_000) { hub.devices.first { invite.deviceId in it }.getValue(invite.deviceId) }
            withTimeout(10_000) { device.state.first { it.isReachable } }
            assertTrue(device.requestPairingByCode(invite), "the PC's certificate should match the code")
            withTimeout(10_000) { device.state.first { it.pairState == PairState.PAIRED } }
            assertEquals(PairState.PAIRED, device.state.value.pairState)
            device.unpair()
        } finally {
            hub.stop()
            scope.cancel()
            dir.deleteRecursively()
        }
    }
}
